//! A sans-IO loopback host for `DashClient`: serves the files of
//! `fixtures/dash/**` by URL and byte range, with fault injection, and drives
//! the client to a terminal state the way an adapter would (zenith-style
//! loopback servers of `broadcast-client/tests/support`).

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};

use broadcast_common::Unpackage;
use dash_runtime::client::{
    Action, DashClient, DashConfig, Error, Failure, FailureClass, FatalKind, Output, PullLimits,
    ResourceId,
};
use transmux::{Fmp4Demux, Sample};
use url::Url;

pub const HOST: &str = "dash.test";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Req {
    pub url: String,
    pub range: Option<(u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum FaultKind {
    Status(u16),
    Transport,
    Timeout,
    Truncate(usize),
    HeaderOnly,
    Garbage,
    ShortRange(usize),
    IgnoreRange,
    Replace(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Fault {
    pub needle: String,
    pub kind: FaultKind,
    pub times: Option<usize>,
    pub offset: Option<u64>,
}

impl Fault {
    pub fn always(needle: &str, kind: FaultKind) -> Self {
        Self {
            needle: needle.to_owned(),
            kind,
            times: None,
            offset: None,
        }
    }

    pub fn at(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn times(needle: &str, kind: FaultKind, times: usize) -> Self {
        Self {
            needle: needle.to_owned(),
            kind,
            times: Some(times),
            offset: None,
        }
    }
}

#[derive(Default)]
pub struct Origin {
    docs: BTreeMap<String, Vec<u8>>,
    mounts: BTreeMap<String, String>,
    faults: Vec<Fault>,
}

impl Origin {
    pub fn new() -> Self {
        let mut o = Self::default();
        o.mounts.insert(HOST.to_owned(), String::new());
        o
    }

    pub fn mount(mut self, host: &str, dir: &str) -> Self {
        self.mounts.insert(host.to_owned(), dir.to_owned());
        self
    }

    pub fn doc(mut self, url: &str, bytes: Vec<u8>) -> Self {
        self.docs.insert(url.to_owned(), bytes);
        self
    }

    pub fn fault(mut self, f: Fault) -> Self {
        self.faults.push(f);
        self
    }

    fn bytes(&self, url: &str) -> Option<Vec<u8>> {
        if let Some(b) = self.docs.get(url) {
            return Some(b.clone());
        }
        let u = Url::parse(url).ok()?;
        let dir = self.mounts.get(u.host_str()?)?;
        let rel = u.path().trim_start_matches('/');
        let rel = if dir.is_empty() {
            format!("dash/{rel}")
        } else {
            format!("dash/{dir}/{rel}")
        };
        let path = test_fixtures::path(&rel);
        std::fs::read(path).ok()
    }

    pub fn serve(&mut self, req: &Req) -> Result<Resp, Failure> {
        let mut fault = None;
        for f in &mut self.faults {
            if !req.url.contains(&f.needle) {
                continue;
            }
            if f.offset.is_some() && f.offset != req.range.map(|r| r.0) {
                continue;
            }
            match &mut f.times {
                Some(0) => continue,
                Some(n) => *n -= 1,
                None => {}
            }
            fault = Some(f.kind.clone());
            break;
        }
        if let Some(FaultKind::Transport) = fault {
            return Err(Failure::Transport);
        }
        if let Some(FaultKind::Timeout) = fault {
            return Err(Failure::Timeout);
        }
        if let Some(FaultKind::Status(s)) = fault {
            return Ok(Resp {
                status: s,
                body: b"error".to_vec(),
            });
        }
        let Some(all) = self.bytes(&req.url) else {
            return Ok(Resp {
                status: 404,
                body: b"not found".to_vec(),
            });
        };
        if let Some(FaultKind::Replace(b)) = &fault {
            return Ok(Resp {
                status: 200,
                body: b.clone(),
            });
        }
        let range = if matches!(fault, Some(FaultKind::IgnoreRange)) {
            None
        } else {
            req.range
        };
        let (status, mut body) = match range {
            Some((off, len)) => {
                let off = off as usize;
                if off >= all.len() {
                    return Ok(Resp {
                        status: 416,
                        body: Vec::new(),
                    });
                }
                let end = (off + len as usize).min(all.len());
                (206, all[off..end].to_vec())
            }
            None => (200, all),
        };
        match fault {
            Some(FaultKind::Truncate(n)) => body.truncate(body.len().saturating_sub(n)),
            Some(FaultKind::HeaderOnly) => body.truncate(8),
            Some(FaultKind::Garbage) => {
                body = b"<html><body>captive portal</body></html>".to_vec();
            }
            Some(FaultKind::ShortRange(n)) => body.truncate(body.len().saturating_sub(n)),
            _ => {}
        }
        Ok(Resp { status, body })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Fifo,
    Lifo,
    Alternate,
}

pub type Hook = Box<dyn FnMut(&mut DashClient, usize)>;

pub struct Drive {
    pub dash: DashConfig,
    pub order: Order,
    pub max_steps: usize,
    pub retry: bool,
    pub skip_instead: bool,
    pub hook: Option<Hook>,
}

impl Default for Drive {
    fn default() -> Self {
        Self {
            order: Order::Fifo,
            dash: DashConfig::default(),
            max_steps: 20_000,
            retry: true,
            skip_instead: false,
            hook: None,
        }
    }
}

pub struct Run {
    pub outputs: Vec<Output>,
    pub requests: Vec<Req>,
    pub errors: Vec<Error>,
    pub classes: Vec<FailureClass>,
    pub max_segments_outstanding: usize,
    pub ended: bool,
    pub fatal: Option<FatalKind>,
    pub stuck: bool,
}

impl Run {
    pub fn samples(&self) -> BTreeMap<u32, Vec<Sample>> {
        let mut out: BTreeMap<u32, Vec<Sample>> = BTreeMap::new();
        for o in &self.outputs {
            if let Output::Samples { track_id, samples } = o {
                out.entry(track_id.value())
                    .or_default()
                    .extend(samples.iter().cloned());
            }
        }
        out
    }

    pub fn count(&self, pred: impl Fn(&Output) -> bool) -> usize {
        self.outputs.iter().filter(|o| pred(o)).count()
    }

    pub fn requested(&self, needle: &str) -> usize {
        self.requests
            .iter()
            .filter(|r| r.url.contains(needle))
            .count()
    }
}

pub fn drive(url: &str, cfg: PullLimits, origin: &mut Origin, opts: &mut Drive) -> Run {
    let mut client = DashClient::new(url, opts.dash, cfg);
    let mut run = Run {
        outputs: Vec::new(),
        requests: Vec::new(),
        errors: Vec::new(),
        classes: Vec::new(),
        max_segments_outstanding: 0,
        ended: false,
        fatal: None,
        stuck: false,
    };
    let mut pending: VecDeque<Action> = VecDeque::new();
    let mut now = 0u64;
    let mut flip = false;
    for step in 0..opts.max_steps {
        if let Some(h) = opts.hook.as_mut() {
            h(&mut client, step);
        }
        while let Some(a) = client.poll(now) {
            pending.push_back(a);
        }
        let segs = pending
            .iter()
            .filter(|a| matches!(a, Action::FetchSegment { .. }))
            .count();
        run.max_segments_outstanding = run.max_segments_outstanding.max(segs);
        while let Some(o) = client.next_output() {
            note(&mut run, o);
        }
        if pending.is_empty() {
            break;
        }
        let idx = match opts.order {
            Order::Fifo => 0,
            Order::Lifo => pending.len() - 1,
            Order::Alternate => {
                flip = !flip;
                if flip { pending.len() - 1 } else { 0 }
            }
        };
        let action = pending.remove(idx).expect("pending");
        now += 1_000;
        let req = Req {
            url: action.url().to_owned(),
            range: action.byte_range(),
        };
        run.requests.push(req.clone());
        let answer = origin.serve(&req);
        deliver(&mut client, &mut run, &action, answer, now, &*opts);
        while let Some(o) = client.next_output() {
            note(&mut run, o);
        }
    }
    run.stuck = !pending.is_empty() && !client.presentation_ended();
    while let Some(o) = client.next_output() {
        note(&mut run, o);
    }
    run
}

fn note(run: &mut Run, o: Output) {
    match &o {
        Output::EndOfStream => run.ended = true,
        Output::Fatal(k) => run.fatal = Some(k.clone()),
        _ => {}
    }
    run.outputs.push(o);
}

fn failure_of(resp: &Resp) -> Option<Failure> {
    (!(200..300).contains(&resp.status)).then_some(Failure::Status(resp.status))
}

fn deliver(
    client: &mut DashClient,
    run: &mut Run,
    action: &Action,
    answer: Result<Resp, Failure>,
    now: u64,
    opts: &Drive,
) {
    let Some(id) = action.id() else {
        match answer {
            Ok(r) if failure_of(&r).is_none() => {
                if let Err(e) = client.on_mpd(now, &r.body) {
                    if matches!(e, Error::Mpd(_)) {
                        client.retry_mpd(now);
                    }
                    run.errors.push(e);
                }
            }
            _ => {
                client.on_mpd_failure(now, Failure::Transport);
                client.retry_mpd(now);
            }
        }
        return;
    };
    let result = match answer {
        Err(f) => Err(f),
        Ok(r) => match failure_of(&r) {
            Some(f) => Err(f),
            None => Ok(r.body),
        },
    };
    match result {
        Err(f) => match client.on_failure(now, id, f) {
            Ok(class) => {
                run.classes.push(class);
                if let FailureClass::Retry { .. } = class {
                    if opts.skip_instead {
                        client.skip(now, id);
                    } else if opts.retry {
                        client.retry(now, id);
                    }
                }
            }
            Err(e) => run.errors.push(e),
        },
        Ok(body) => {
            let r = match id {
                ResourceId::Init { .. } => client.on_init(now, id, &body),
                ResourceId::Index { .. } => client.on_index(now, id, &body),
                ResourceId::Segment { .. } => client.on_segment(now, id, &body),
                _ => Ok(()),
            };
            if let Err(e) = r {
                if let Error::Rejected { id, class, .. } = &e {
                    run.classes.push(*class);
                    if let FailureClass::Retry { .. } = class {
                        if opts.skip_instead {
                            client.skip(now, *id);
                        } else if opts.retry {
                            client.retry(now, *id);
                        }
                    }
                }
                run.errors.push(e);
            }
        }
    }
}

pub fn play(mpd: &str, cfg: PullLimits) -> Run {
    drive(
        &format!("http://{HOST}/{mpd}"),
        cfg,
        &mut Origin::new(),
        &mut Drive::default(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sig {
    pub data: Vec<u8>,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub duration: Option<u32>,
    pub flags: String,
}

pub fn sig(s: &Sample) -> Sig {
    Sig {
        data: s.data.to_vec(),
        pts: s.pts,
        dts: s.dts,
        duration: s.duration,
        flags: format!("{:?}", s.flags),
    }
}

pub fn sigs(samples: &[Sample]) -> Vec<Sig> {
    samples.iter().map(sig).collect()
}

/// The samples `Fmp4Demux` produces from the concatenation of `files` (an init
/// first), per track in the init's order. This is the oracle: it never sees
/// the client.
pub fn oracle(files: &[&str]) -> Vec<Vec<Sig>> {
    let mut bytes = Vec::new();
    for f in files {
        bytes.extend_from_slice(&test_fixtures::read(&format!("dash/{f}")));
    }
    let media = Fmp4Demux::new()
        .unpackage(bytes.as_slice())
        .expect("oracle demux");
    media.tracks.iter().map(|t| sigs(&t.samples)).collect()
}

pub fn assert_matches_oracle(run: &Run, expected_by_track: &[Vec<Sig>]) {
    let got = run.samples();
    assert_eq!(
        got.len(),
        expected_by_track.len(),
        "track count: got ids {:?}",
        got.keys().collect::<Vec<_>>()
    );
    for (i, want) in expected_by_track.iter().enumerate() {
        let id = i as u32 + 1;
        let have = sigs(got.get(&id).unwrap_or_else(|| panic!("track {id} missing")));
        assert_eq!(have.len(), want.len(), "track {id} sample count");
        for (n, (h, w)) in have.iter().zip(want).enumerate() {
            assert_eq!(h, w, "track {id} sample {n}");
        }
    }
}

pub fn mpd_text(rel: &str) -> String {
    test_fixtures::read_to_string(&format!("dash/{rel}"))
}

pub fn fatal_status(run: &Run) -> Option<u16> {
    match &run.fatal {
        Some(FatalKind::SegmentFetchFailed { status, .. })
        | Some(FatalKind::InitFetchFailed { status, .. }) => *status,
        _ => None,
    }
}
