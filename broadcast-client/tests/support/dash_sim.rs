//! A simulated host for a DASH `Session`: true time and a host clock with skew, drift and steps
//! (the `World` of the core suites), an origin that answers by true time, per-request faults, and
//! the callbacks a real host makes (`on_response`, `on_data`, `on_closed`). No clock is read.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use broadcast_client::{Action, Config, Event, Session, Status};
use transmux::Sample;

use super::dash_common::loopback;
use super::dash_common::world::{self, HttpReq, Method, Outcome, World};

pub type Headers = Vec<(String, String)>;

pub const SIM_ETAG: &str = "\"sim-v1\"";
pub const MS: u64 = 1_000;
const MAX_STEPS: u64 = 3_000_000;
pub const SEC: u64 = 1_000_000;

#[derive(Debug, Clone)]
pub struct Logged {
    pub conn: u32,
    pub method: String,
    pub url: String,
    pub headers: Headers,
    pub byte_range: Option<(u64, u64)>,
    pub send_elapsed_us: u64,
    pub send_mono_us: u64,
    pub send_utc_us: i64,
    pub status: Option<u16>,
}

impl Logged {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn is_mpd(&self) -> bool {
        self.url
            .split('?')
            .next()
            .is_some_and(|p| p.ends_with(".mpd"))
    }
}

#[derive(Debug, Clone)]
pub enum Answer {
    Full {
        status: u16,
        headers: Headers,
        body: Vec<u8>,
    },
    Redirect(String),
    TransportError,
    ResetMid(Vec<u8>),
    Truncated {
        body: Vec<u8>,
        declared: usize,
    },
    Stall(Vec<u8>),
    StallWith {
        status: u16,
        headers: Headers,
        body: Vec<u8>,
    },
    Hang,
    Late(Box<Answer>, u64),
    Duplicate(Box<Answer>),
    Unknown(Box<Answer>),
    ClosedTwice(Box<Answer>),
}

impl Answer {
    pub fn ok(body: Vec<u8>) -> Self {
        Self::Full {
            status: 200,
            headers: Vec::new(),
            body,
        }
    }

    pub fn status(status: u16) -> Self {
        Self::Full {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn status_with(status: u16, headers: Headers) -> Self {
        Self::Full {
            status,
            headers,
            body: Vec::new(),
        }
    }

    pub fn with_header(self, name: &str, value: &str) -> Self {
        match self {
            Self::Full {
                status,
                mut headers,
                body,
            } => {
                headers.push((name.to_string(), value.to_string()));
                Self::Full {
                    status,
                    headers,
                    body,
                }
            }
            other => other,
        }
    }
}

fn parse_open_range(value: &str) -> Option<(u64, Option<u64>)> {
    let spec = value.strip_prefix("bytes=")?;
    let (first, last) = spec.split_once('-')?;
    Some((first.parse().ok()?, last.parse().ok()))
}

/// What an origin that honours `Range` and `If-Range` (RFC 9110 14.2, 13.1.5) makes of `answer`:
/// the requested slice as a `206`, or the untouched `200` when `If-Range` names another validator.
pub fn range_answer(answer: Answer, range: &str, if_range: Option<&str>) -> Answer {
    let Answer::Full {
        status: 200,
        mut headers,
        body,
    } = answer
    else {
        return answer;
    };
    let Some((first, last)) = parse_open_range(range) else {
        return Answer::Full {
            status: 200,
            headers,
            body,
        };
    };
    let etag = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("etag"))
        .map_or_else(|| SIM_ETAG.to_string(), |(_, v)| v.clone());
    let total = body.len() as u64;
    let last = last.map_or(total.saturating_sub(1), |l| l.min(total.saturating_sub(1)));
    if if_range.is_some_and(|validator| validator != etag) || first > last {
        return Answer::Full {
            status: 200,
            headers,
            body,
        };
    }
    headers.retain(|(k, _)| {
        !["etag", "content-length", "content-range"]
            .iter()
            .any(|n| k.eq_ignore_ascii_case(n))
    });
    let slice = body[usize::try_from(first).unwrap()..=usize::try_from(last).unwrap()].to_vec();
    headers.push(("ETag".into(), etag));
    headers.push(("Accept-Ranges".into(), "bytes".into()));
    headers.push((
        "Content-Range".into(),
        format!("bytes {first}-{last}/{total}"),
    ));
    headers.push(("Content-Length".into(), slice.len().to_string()));
    Answer::Full {
        status: 206,
        headers,
        body: slice,
    }
}

/// `good` turned into a transfer that sends its head and its first `sent` bytes and then goes
/// silent, carrying the validators a resumable origin sends.
pub fn stall_after(good: &Answer, sent: usize) -> Option<Answer> {
    let Answer::Full {
        status: status @ (200 | 206),
        headers,
        body,
    } = good
    else {
        return None;
    };
    let mut headers = headers.clone();
    headers.retain(|(k, _)| {
        !["etag", "accept-ranges", "content-length"]
            .iter()
            .any(|n| k.eq_ignore_ascii_case(n))
    });
    headers.push(("ETag".into(), SIM_ETAG.into()));
    headers.push(("Accept-Ranges".into(), "bytes".into()));
    headers.push(("Content-Length".into(), body.len().to_string()));
    Some(Answer::StallWith {
        status: *status,
        headers,
        body: body[..sent.min(body.len())].to_vec(),
    })
}

pub trait Backend {
    fn serve(&mut self, at_utc_us: i64, req: &Logged) -> Answer;
}

impl<F: FnMut(i64, &Logged) -> Answer> Backend for F {
    fn serve(&mut self, at_utc_us: i64, req: &Logged) -> Answer {
        self(at_utc_us, req)
    }
}

pub struct WorldBackend<O: world::Origin>(pub O);

impl<O: world::Origin> Backend for WorldBackend<O> {
    fn serve(&mut self, at_utc_us: i64, req: &Logged) -> Answer {
        let method = if req.method == "HEAD" {
            Method::Head
        } else {
            Method::Get
        };
        let http = HttpReq {
            method,
            url: req.url.clone(),
            range: req.byte_range,
            if_none_match: req.header("if-none-match").map(str::to_owned),
        };
        match self.0.serve(at_utc_us, &http) {
            Outcome::Resp(r) => {
                let mut headers = Headers::new();
                if let Some(d) = r.date {
                    headers.push(("Date".into(), d));
                }
                if let Some(e) = r.etag {
                    headers.push(("ETag".into(), e));
                }
                if let Some(loc) = r.final_url {
                    return Answer::Redirect(loc);
                }
                Answer::Full {
                    status: r.status,
                    headers,
                    body: r.body,
                }
            }
            Outcome::Fail(dash_runtime::client::Failure::Transport) => Answer::TransportError,
            Outcome::Fail(dash_runtime::client::Failure::Status(s)) => Answer::status(s),
            Outcome::Fail(_) | Outcome::Hang => Answer::Hang,
        }
    }
}

pub struct StaticBackend(pub loopback::Origin);

impl Backend for StaticBackend {
    fn serve(&mut self, _at_utc_us: i64, req: &Logged) -> Answer {
        let lreq = loopback::Req {
            url: req.url.clone(),
            range: req.byte_range,
        };
        match self.0.serve(&lreq) {
            Err(dash_runtime::client::Failure::Transport) => Answer::TransportError,
            Err(_) => Answer::Hang,
            Ok(resp) => {
                let mut headers = Headers::new();
                if resp.status == 206
                    && let Some((off, len)) = req.byte_range
                {
                    headers.push((
                        "Content-Range".into(),
                        format!("bytes {off}-{}/*", off + len.saturating_sub(1)),
                    ));
                }
                Answer::Full {
                    status: resp.status,
                    headers,
                    body: resp.body,
                }
            }
        }
    }
}

enum Delivery {
    Answer { conn: u32, answer: Answer },
}

struct Pending {
    at: u64,
    seq: u64,
    conn: u32,
    delivery: Delivery,
}

pub type FaultFn = Box<dyn FnMut(&Logged, i64) -> Option<Answer>>;

pub struct DashSim {
    pub session: Session,
    pub world: World,
    pub backend: Box<dyn Backend>,
    pending: Vec<Pending>,
    seq: u64,
    wakes: BTreeSet<u64>,
    pub log: Vec<Logged>,
    pub events: Vec<Event>,
    pub event_times: Vec<u64>,
    pub closes: Vec<u32>,
    pub close_log: Vec<(u32, u64)>,
    pub rtt_us: u64,
    pub chunk: usize,
    pub fault: Option<FaultFn>,
    pub honour_ranges: bool,
    pub request_cap: usize,
    pub stalls: u64,
    pub steps: u64,
    pub host_clock_calls: Vec<(u64, u64)>,
}

impl DashSim {
    pub fn new(url: &str, config: Config, world: World, backend: Box<dyn Backend>) -> Self {
        let session = Session::open(url, config, world.mono()).expect("open");
        Self::with_session(session, world, backend)
    }

    pub fn with_session(session: Session, world: World, backend: Box<dyn Backend>) -> Self {
        Self {
            session,
            world,
            backend,
            pending: Vec::new(),
            seq: 0,
            wakes: BTreeSet::new(),
            log: Vec::new(),
            events: Vec::new(),
            event_times: Vec::new(),
            closes: Vec::new(),
            close_log: Vec::new(),
            rtt_us: 40 * MS,
            chunk: 65_536,
            fault: None,
            honour_ranges: true,
            request_cap: 20_000,
            stalls: 0,
            steps: 0,
            host_clock_calls: Vec::new(),
        }
    }

    pub fn static_origin(url: &str, config: Config) -> Self {
        Self::new(
            url,
            config,
            World::new(1_700_000_000 * 1_000_000),
            Box::new(StaticBackend(loopback::Origin::new())),
        )
    }

    pub fn give_host_clock(&mut self) {
        let mono = self.world.mono();
        let unix = self.world.host_unix();
        self.session.set_wall_clock(mono, unix);
        self.host_clock_calls.push((mono, unix));
    }

    fn drain_events(&mut self) {
        while let Some(e) = self.session.next_event() {
            self.events.push(e);
            self.event_times.push(self.world.elapsed_us);
        }
    }

    fn dispatch(
        &mut self,
        conn: u32,
        method: String,
        url: String,
        headers: Headers,
        range: Option<(u64, u64)>,
    ) {
        let sent_elapsed = self.world.elapsed_us;
        let logged = Logged {
            conn,
            method,
            url,
            headers,
            byte_range: range,
            send_elapsed_us: sent_elapsed,
            send_mono_us: self.world.mono(),
            send_utc_us: self.world.true_utc_at(sent_elapsed),
            status: None,
        };
        let origin_utc = self.world.true_utc_at(sent_elapsed + self.rtt_us / 2);
        let arrive = sent_elapsed + self.rtt_us;
        let mut answer = None;
        if let Some(f) = self.fault.as_mut() {
            answer = f(&logged, origin_utc);
        }
        let mut answer = answer.unwrap_or_else(|| self.backend.serve(origin_utc, &logged));
        if self.honour_ranges
            && let Some(range) = logged.header("range")
        {
            answer = range_answer(answer, range, logged.header("if-range"));
        }
        let mut logged = logged;
        logged.status = match &answer {
            Answer::Full { status, .. } => Some(*status),
            Answer::Redirect(_) => Some(302),
            Answer::StallWith { status, .. } => Some(*status),
            Answer::Late(inner, _)
            | Answer::Duplicate(inner)
            | Answer::Unknown(inner)
            | Answer::ClosedTwice(inner) => match inner.as_ref() {
                Answer::Full { status, .. } => Some(*status),
                _ => None,
            },
            _ => None,
        };
        self.log.push(logged);
        match answer {
            Answer::Hang => {}
            Answer::Late(inner, delay) => self.push(arrive + delay, conn, *inner),
            other => self.push(arrive, conn, other),
        }
    }

    fn push(&mut self, at: u64, conn: u32, answer: Answer) {
        self.seq += 1;
        self.pending.push(Pending {
            at,
            seq: self.seq,
            conn,
            delivery: Delivery::Answer { conn, answer },
        });
    }

    pub fn poll_all(&mut self) {
        loop {
            let now = self.world.mono();
            let Some(a) = self.session.poll(now) else {
                break;
            };
            match a {
                Action::HttpRequest {
                    conn,
                    method,
                    url,
                    headers,
                    byte_range,
                    ..
                } => self.dispatch(conn, method, url, headers, byte_range),
                Action::Close { conn } => {
                    self.pending.retain(|p| p.conn != conn);
                    self.closes.push(conn);
                    self.close_log.push((conn, self.world.elapsed_us));
                }
                Action::WakeAt(t) if t > now => {
                    self.wakes.insert(t);
                }
                _ => {}
            }
        }
        self.drain_events();
    }

    fn respond(&mut self, conn: u32, answer: Answer) {
        let now = self.world.mono();
        let chunk = self.chunk.max(1);
        match answer {
            Answer::Full {
                status,
                mut headers,
                body,
            } => {
                if !headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                {
                    headers.push(("Content-Length".into(), body.len().to_string()));
                }
                self.session.on_response(conn, status, &headers, now);
                for c in body.chunks(chunk) {
                    self.session.on_data(conn, c, now);
                }
                self.session.on_closed(conn, None, now);
            }
            Answer::Redirect(loc) => {
                self.session
                    .on_response(conn, 302, &[("Location".into(), loc)], now);
            }
            Answer::TransportError => self.session.on_closed(conn, Some("reset".into()), now),
            Answer::ResetMid(body) => {
                self.session.on_response(conn, 200, &[], now);
                for c in body.chunks(chunk) {
                    self.session.on_data(conn, c, now);
                }
                self.session.on_closed(conn, Some("reset".into()), now);
            }
            Answer::Truncated { body, declared } => {
                self.session.on_response(
                    conn,
                    200,
                    &[("Content-Length".into(), declared.to_string())],
                    now,
                );
                for c in body.chunks(chunk) {
                    self.session.on_data(conn, c, now);
                }
                self.session.on_closed(conn, None, now);
            }
            Answer::Stall(body) => {
                self.session.on_response(conn, 200, &[], now);
                for c in body.chunks(chunk) {
                    self.session.on_data(conn, c, now);
                }
            }
            Answer::StallWith {
                status,
                headers,
                body,
            } => {
                self.session.on_response(conn, status, &headers, now);
                for c in body.chunks(chunk) {
                    self.session.on_data(conn, c, now);
                }
            }
            Answer::Duplicate(inner) => {
                let replay = match inner.as_ref() {
                    Answer::Full { body, .. } => body[..body.len().min(1000)].to_vec(),
                    _ => Vec::new(),
                };
                self.respond(conn, *inner);
                self.session.on_response(conn, 200, &[], now);
                self.session.on_data(conn, &replay, now);
                self.session.on_closed(conn, None, now);
            }
            Answer::Unknown(inner) => {
                const STRANGER: u32 = 9_999;
                self.session.on_response(STRANGER, 200, &[], now);
                self.session.on_data(STRANGER, b"xx", now);
                self.session.on_closed(STRANGER, None, now);
                self.respond(conn, *inner);
            }
            Answer::ClosedTwice(inner) => {
                self.respond(conn, *inner);
                self.session.on_closed(conn, None, now);
                self.session.on_closed(conn, Some("x".into()), now);
            }
            Answer::Hang | Answer::Late(..) => {}
        }
    }

    pub fn idle(&self) -> bool {
        self.pending.is_empty() && self.wakes.is_empty()
    }

    pub fn terminal(&self) -> bool {
        self.events.iter().any(|e| {
            matches!(
                e,
                Event::Status(Status::Ended) | Event::Error { fatal: true, .. }
            )
        })
    }

    fn run_loop(&mut self, end_elapsed_us: u64, stop_when_terminal: bool) {
        let mut stall_run = 0u64;
        loop {
            self.steps += 1;
            assert!(
                self.steps < MAX_STEPS,
                "the simulation did not settle in {MAX_STEPS} steps"
            );
            self.poll_all();
            if (stop_when_terminal && self.terminal()) || self.log.len() > self.request_cap {
                break;
            }
            let mut next = end_elapsed_us;
            for p in &self.pending {
                next = next.min(p.at);
            }
            if let Some(w) = self.wakes.iter().next().copied() {
                next = next.min(self.world.elapsed_for_mono(w));
            }
            if next <= self.world.elapsed_us {
                if next >= end_elapsed_us {
                    break;
                }
                let due = self.pending.iter().any(|p| p.at <= self.world.elapsed_us);
                if !due {
                    stall_run += 1;
                    self.stalls += 1;
                    next = self.world.elapsed_us + MS;
                } else {
                    stall_run = 0;
                }
            } else {
                stall_run = 0;
            }
            let _ = stall_run;
            self.world.elapsed_us = next.min(end_elapsed_us);
            let now_mono = self.world.mono();
            let expired: Vec<u64> = self
                .wakes
                .iter()
                .copied()
                .filter(|w| *w <= now_mono)
                .collect();
            for w in expired {
                self.wakes.remove(&w);
            }
            let mut due: Vec<Pending> = Vec::new();
            let mut rest = Vec::new();
            for p in self.pending.drain(..) {
                if p.at <= self.world.elapsed_us {
                    due.push(p);
                } else {
                    rest.push(p);
                }
            }
            self.pending = rest;
            due.sort_by_key(|p| (p.at, p.seq));
            for p in due {
                let Delivery::Answer { conn, answer } = p.delivery;
                self.respond(conn, answer);
                self.drain_events();
            }
            if self.world.elapsed_us >= end_elapsed_us {
                self.poll_all();
                break;
            }
            if self.pending.is_empty() && self.wakes.is_empty() {
                self.poll_all();
                if self.pending.is_empty() && self.wakes.is_empty() {
                    break;
                }
            }
        }
    }

    /// The host sleeps: time passes with no call into the session.
    pub fn sleep(&mut self, dur_us: u64) {
        self.world.elapsed_us += dur_us;
    }

    pub fn run_until(&mut self, end_elapsed_us: u64) {
        self.run_loop(end_elapsed_us, false);
    }

    pub fn run_for(&mut self, dur_us: u64) {
        let end = self.world.elapsed_us + dur_us;
        self.run_until(end);
    }

    pub fn run_to_terminal(&mut self, max_us: u64) {
        let end = self.world.elapsed_us + max_us;
        self.run_loop(end, true);
    }

    pub fn samples(&self) -> BTreeMap<u32, Vec<Sample>> {
        let mut out: BTreeMap<u32, Vec<Sample>> = BTreeMap::new();
        for e in &self.events {
            if let Event::Sample { track, sample, .. } = e {
                out.entry(track.value()).or_default().push(sample.clone());
            }
        }
        out
    }

    pub fn errors(&self) -> Vec<(broadcast_client::ErrorKind, bool)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::Error { kind, fatal } => Some((kind.clone(), *fatal)),
                _ => None,
            })
            .collect()
    }

    pub fn error_names(&self) -> Vec<&'static str> {
        self.errors().iter().map(|(k, _)| k.name()).collect()
    }

    pub fn fatal(&self) -> Option<broadcast_client::ErrorKind> {
        self.errors().into_iter().find(|(_, f)| *f).map(|(k, _)| k)
    }

    pub fn statuses(&self) -> Vec<Status> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::Status(s) => Some(*s),
                _ => None,
            })
            .collect()
    }

    pub fn ended(&self) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Ended)))
    }

    pub fn count(&self, pred: impl Fn(&Event) -> bool) -> usize {
        self.events.iter().filter(|e| pred(e)).count()
    }

    pub fn requests_to(&self, needle: &str) -> Vec<&Logged> {
        self.log.iter().filter(|l| l.url.contains(needle)).collect()
    }
}

pub fn sigs_of(samples: &[Sample]) -> Vec<loopback::Sig> {
    loopback::sigs(samples)
}

/// Assert that the delivered samples of each track equal the oracle's, track by track.
pub fn assert_samples_equal_oracle(sim: &DashSim, expected_by_track: &[Vec<loopback::Sig>]) {
    let got = sim.samples();
    assert_eq!(
        got.len(),
        expected_by_track.len(),
        "track count: got ids {:?}",
        got.keys().collect::<Vec<_>>()
    );
    for (i, want) in expected_by_track.iter().enumerate() {
        let id = u32::try_from(i).unwrap() + 1;
        let have = sigs_of(got.get(&id).unwrap_or_else(|| panic!("track {id} missing")));
        assert_eq!(have.len(), want.len(), "track {id} sample count");
        for (n, (h, w)) in have.iter().zip(want).enumerate() {
            assert_eq!(h, w, "track {id} sample {n}");
        }
    }
}
