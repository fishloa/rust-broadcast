//! Per-request inactivity timeout of the HLS source with `Range` resume (issue #1613), driven
//! through a `Session` on a caller clock over the real TS fixture segments: the timing of the
//! close, the resume request, every answer an origin can give to it, and the cases where nothing
//! may change.

use crate::support;

use std::collections::BTreeSet;

use broadcast_client::{Action, Config, ErrorKind, Event, ResumeMode, Session, Status};
use support::hls_host::{
    Host, MS, SEC, TS_PLAYLIST_URL, TS_SEG0_URL, TS_SEG1_URL, hls_config, ts_playlist, ts_segment,
};

const ETAG: &str = "\"seg-v1\"";
const OTHER_ETAG: &str = "\"seg-v2\"";
const WEAK_ETAG: &str = "W/\"seg-v1\"";
const INACTIVITY_US: u64 = 3 * SEC;
const FIRST_BACKOFF_US: u64 = 2 * SEC;
const TIME_CAP_US: u64 = 3_600 * SEC;
const BYTERANGE_URL: &str = "http://origin.test/hls/all.ts";

type Headers = Vec<(String, String)>;

#[derive(Clone, Debug)]
struct Req {
    conn: u32,
    url: String,
    headers: Headers,
    byte_range: Option<(u64, u64)>,
    at_us: u64,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn h(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[derive(Clone, Debug)]
enum Serve {
    Whole(u16, Headers, Vec<u8>),
    Stall(u16, Headers, Vec<u8>),
}

struct Drive {
    session: Session,
    now: u64,
    pending: Vec<Req>,
    wakes: BTreeSet<u64>,
    events: Vec<Event>,
    log: Vec<Req>,
    closes: Vec<(u32, u64)>,
}

impl Drive {
    fn new(config: Config) -> Self {
        Self {
            session: Session::open(TS_PLAYLIST_URL, config, 0).expect("open"),
            now: 0,
            pending: Vec::new(),
            wakes: BTreeSet::new(),
            events: Vec::new(),
            log: Vec::new(),
            closes: Vec::new(),
        }
    }

    fn collect(&mut self) {
        while let Some(action) = self.session.poll(self.now) {
            match action {
                Action::HttpRequest {
                    conn,
                    url,
                    headers,
                    byte_range,
                    ..
                } => {
                    let req = Req {
                        conn,
                        url,
                        headers,
                        byte_range,
                        at_us: self.now,
                    };
                    self.log.push(req.clone());
                    self.pending.push(req);
                }
                Action::Close { conn } => {
                    self.closes.push((conn, self.now));
                    self.pending.retain(|r| r.conn != conn);
                }
                Action::WakeAt(t) if t > self.now => {
                    self.wakes.insert(t);
                }
                _ => {}
            }
        }
        while let Some(event) = self.session.next_event() {
            self.events.push(event);
        }
    }

    fn deliver(&mut self, conn: u32, serve: Serve) {
        match serve {
            Serve::Whole(status, headers, body) => {
                self.session.on_response(conn, status, &headers, self.now);
                if !body.is_empty() {
                    self.session.on_data(conn, &body, self.now);
                }
                self.session.on_closed(conn, None, self.now);
            }
            Serve::Stall(status, headers, prefix) => {
                self.session.on_response(conn, status, &headers, self.now);
                if !prefix.is_empty() {
                    self.session.on_data(conn, &prefix, self.now);
                }
            }
        }
    }

    fn finished(&self) -> bool {
        self.events.iter().any(|e| {
            matches!(
                e,
                Event::Status(Status::Ended) | Event::Error { fatal: true, .. }
            )
        })
    }

    fn run(&mut self, serve: &mut dyn FnMut(&Req) -> Serve) {
        loop {
            self.collect();
            if self.finished() {
                return;
            }
            if let Some(req) = self.pending.pop() {
                let answer = serve(&req);
                self.deliver(req.conn, answer);
                continue;
            }
            let Some(next) = self.wakes.range(self.now + 1..).next().copied() else {
                return;
            };
            assert!(next < TIME_CAP_US, "the session never finished");
            self.wakes.remove(&next);
            self.now = next;
        }
    }

    fn samples(&self) -> Vec<Fingerprint> {
        samples_of(&self.events)
    }

    fn statuses(&self) -> Vec<Status> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::Status(s) => Some(*s),
                _ => None,
            })
            .collect()
    }

    fn stalls(&self) -> Vec<ResumeMode> {
        self.statuses()
            .into_iter()
            .filter_map(|s| match s {
                Status::RequestStalled { resume } => Some(resume),
                _ => None,
            })
            .collect()
    }

    fn closed_at(&self, conn: u32) -> Option<u64> {
        self.closes
            .iter()
            .find(|(c, _)| *c == conn)
            .map(|(_, t)| *t)
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

type Fingerprint = (u32, String, usize, u64);

fn samples_of(events: &[Event]) -> Vec<Fingerprint> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } => {
                let digest = sample.data.iter().fold(FNV_OFFSET, |h, b| {
                    (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
                });
                Some((
                    track.value(),
                    format!("{:?}", sample.dts),
                    sample.data.len(),
                    digest,
                ))
            }
            _ => None,
        })
        .collect()
}

fn config() -> Config {
    let mut config = hls_config();
    config.segment_inactivity_timeout_us = INACTIVITY_US;
    config
}

fn baseline_samples() -> Vec<Fingerprint> {
    let mut host = Host::new(TS_PLAYLIST_URL, hls_config());
    host.ok(TS_PLAYLIST_URL, ts_playlist());
    host.ok(TS_SEG0_URL, ts_segment(0));
    host.ok(TS_SEG1_URL, ts_segment(1));
    host.run_to_end();
    let samples = samples_of(&host.events);
    assert!(!samples.is_empty());
    samples
}

fn range_of(value: &str) -> (u64, Option<u64>) {
    let spec = value.strip_prefix("bytes=").expect("a bytes range");
    let (first, last) = spec.split_once('-').expect("first-last");
    (first.parse().unwrap(), last.parse().ok())
}

fn representation_headers(total: usize, validator: Option<&str>) -> Headers {
    let mut headers = h(&[("Accept-Ranges", "bytes")]);
    if let Some(v) = validator {
        headers.push(("ETag".into(), v.into()));
    }
    headers.push(("Content-Length".into(), total.to_string()));
    headers
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Honours,
    IgnoresRanges,
    NewValidator,
    WrongContentRange,
    Unsatisfiable,
    NoValidator,
    WeakValidator,
}

macro_rules! origins {
    ($($variant:ident),+ $(,)?) => {
        const ALL_ORIGINS: &[Origin] = &[$(Origin::$variant),+];
    };
}

origins!(
    Honours,
    IgnoresRanges,
    NewValidator,
    WrongContentRange,
    Unsatisfiable,
    NoValidator,
    WeakValidator,
);

impl Origin {
    fn first_validator(self) -> Option<&'static str> {
        match self {
            Origin::NoValidator => None,
            Origin::WeakValidator => Some(WEAK_ETAG),
            _ => Some(ETAG),
        }
    }

    fn resumes(self) -> bool {
        match self {
            Origin::Honours
            | Origin::IgnoresRanges
            | Origin::NewValidator
            | Origin::WrongContentRange
            | Origin::Unsatisfiable => true,
            Origin::NoValidator | Origin::WeakValidator => false,
        }
    }

    fn extra_requests(self) -> usize {
        match self {
            Origin::WrongContentRange | Origin::Unsatisfiable => 1,
            _ => 0,
        }
    }
}

type ViewOf = dyn Fn(&Req) -> (usize, usize);

struct Scenario {
    file: Vec<u8>,
    playlist: Vec<u8>,
    target: Box<dyn Fn(&Req) -> bool>,
    view: Box<ViewOf>,
    origin: Origin,
    sent: usize,
}

fn plain(origin: Origin, sent: usize) -> Scenario {
    let file = ts_segment(0);
    let len = file.len();
    Scenario {
        file,
        playlist: ts_playlist(),
        target: Box::new(|r| r.url == TS_SEG0_URL),
        view: Box::new(move |_| (0, len)),
        origin,
        sent,
    }
}

fn byte_ranged(origin: Origin, sent: usize) -> (Scenario, Vec<Fingerprint>) {
    let (first, second) = (ts_segment(0), ts_segment(1));
    let (len0, len1) = (first.len(), second.len());
    let mut file = first;
    file.extend_from_slice(&second);
    let playlist = format!(
        "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:2.0,\n#EXT-X-BYTERANGE:{len0}@0\nall.ts\n\
         #EXTINF:1.0,\n#EXT-X-BYTERANGE:{len1}@{len0}\nall.ts\n#EXT-X-ENDLIST\n"
    )
    .into_bytes();
    let mut host = Host::new(TS_PLAYLIST_URL, hls_config());
    host.ok(TS_PLAYLIST_URL, playlist.clone());
    host.ok(BYTERANGE_URL, file.clone());
    host.run_to_end();
    let baseline = samples_of(&host.events);
    assert!(!baseline.is_empty());
    let view_file_len = len0;
    let scenario = Scenario {
        file,
        playlist,
        target: Box::new(move |r| {
            r.byte_range.is_some_and(|(o, _)| o == 0)
                || r.header("range")
                    .is_some_and(|v| usize::try_from(range_of(v).0).unwrap() < view_file_len)
        }),
        view: Box::new(move |r| match r.byte_range {
            Some((o, l)) => (usize::try_from(o).unwrap(), usize::try_from(l).unwrap()),
            None => (0, len0 + len1),
        }),
        origin,
        sent,
    };
    (scenario, baseline)
}

impl Scenario {
    fn answer(&self, req: &Req, nth_target: usize) -> Serve {
        let url_is_playlist = req.url == TS_PLAYLIST_URL;
        if url_is_playlist {
            return Serve::Whole(200, Vec::new(), self.playlist.clone());
        }
        let total = self.file.len();
        let (view_off, view_len) = (self.view)(req);
        let whole = |validator: Option<&str>| -> Serve {
            let body = self.file[view_off..view_off + view_len].to_vec();
            match req.byte_range {
                Some((o, l)) => {
                    let mut headers = representation_headers(body.len(), validator);
                    headers.push((
                        "Content-Range".into(),
                        format!("bytes {o}-{}/{total}", o + l - 1),
                    ));
                    Serve::Whole(206, headers, body)
                }
                None => Serve::Whole(200, representation_headers(body.len(), validator), body),
            }
        };
        if !(self.target)(req) {
            if req.url == TS_SEG1_URL {
                let body = ts_segment(1);
                let headers = representation_headers(body.len(), Some(ETAG));
                return Serve::Whole(200, headers, body);
            }
            return whole(Some(ETAG));
        }
        if nth_target == 0 {
            let Serve::Whole(status, headers, body) = whole(self.origin.first_validator()) else {
                unreachable!()
            };
            return Serve::Stall(status, headers, body[..self.sent.min(body.len())].to_vec());
        }
        let Some(range) = req.header("range") else {
            return whole(self.origin.first_validator());
        };
        let (first, last) = range_of(range);
        let first_at = usize::try_from(first).unwrap();
        let last_at = last.map_or(total - 1, |l| usize::try_from(l).unwrap());
        let slice = |etag: &str, first_at: usize, shift: usize| {
            let mut headers = representation_headers(last_at - first_at + 1, Some(etag));
            headers.push((
                "Content-Range".into(),
                format!("bytes {}-{last_at}/{total}", first_at + shift),
            ));
            Serve::Whole(206, headers, self.file[first_at + shift..=last_at].to_vec())
        };
        match self.origin {
            Origin::Honours => {
                assert_eq!(req.header("if-range"), Some(ETAG));
                slice(ETAG, first_at, 0)
            }
            Origin::IgnoresRanges => Serve::Whole(
                200,
                representation_headers(total, Some(ETAG)),
                self.file.clone(),
            ),
            Origin::NewValidator => {
                let body = self.file[view_off..view_off + view_len].to_vec();
                Serve::Whole(
                    200,
                    representation_headers(body.len(), Some(OTHER_ETAG)),
                    body,
                )
            }
            Origin::WrongContentRange => slice(ETAG, first_at, 1),
            Origin::Unsatisfiable => Serve::Whole(416, Vec::new(), Vec::new()),
            Origin::NoValidator | Origin::WeakValidator => {
                panic!("an origin without a strong validator was sent a Range: {req:?}")
            }
        }
    }

    fn run(&self, config: Config) -> Drive {
        let mut drive = Drive::new(config);
        let mut nth_target = 0usize;
        drive.run(&mut |req| {
            let serve = self.answer(req, nth_target);
            if req.url != TS_PLAYLIST_URL && (self.target)(req) {
                nth_target += 1;
            }
            serve
        });
        drive
    }
}

fn target_requests<'d>(drive: &'d Drive, scenario: &Scenario) -> Vec<&'d Req> {
    drive.log.iter().filter(|r| (scenario.target)(r)).collect()
}

#[test]
fn a_silent_segment_closes_at_exactly_the_inactivity_timeout_and_resumes_after_the_backoff() {
    let scenario = plain(Origin::Honours, 5_000);
    let drive = scenario.run(config());
    assert_eq!(drive.samples(), baseline_samples());

    let requests = target_requests(&drive, &scenario);
    assert_eq!(requests.len(), 2);
    let (first, second) = (requests[0], requests[1]);
    let closed = drive.closed_at(first.conn).expect("closed for inactivity");
    assert_eq!(closed, first.at_us + INACTIVITY_US);
    assert_eq!(second.at_us, closed + FIRST_BACKOFF_US);
    let total = scenario.file.len();
    assert_eq!(
        second.header("range"),
        Some(format!("bytes=5000-{}", total - 1).as_str())
    );
    assert_eq!(second.header("if-range"), Some(ETAG));
    assert_eq!(second.byte_range, None);
    assert_eq!(drive.stalls(), vec![ResumeMode::Range]);
    assert!(
        drive
            .statuses()
            .contains(&Status::Reconnecting { attempt: 1 })
    );
}

#[test]
fn nothing_closes_one_microsecond_before_the_timeout() {
    fn collect(session: &mut Session, now: u64) -> (Vec<(u32, String)>, Vec<u32>) {
        let (mut requests, mut closes) = (Vec::new(), Vec::new());
        while let Some(action) = session.poll(now) {
            match action {
                Action::HttpRequest { conn, url, .. } => requests.push((conn, url)),
                Action::Close { conn } => closes.push(conn),
                _ => {}
            }
        }
        (requests, closes)
    }
    let mut session = Session::open(TS_PLAYLIST_URL, config(), 0).unwrap();
    let (opened, _) = collect(&mut session, 0);
    let playlist_conn = opened[0].0;
    let playlist = ts_playlist();
    session.on_response(playlist_conn, 200, &[], 0);
    session.on_data(playlist_conn, &playlist, 0);
    session.on_closed(playlist_conn, None, 0);
    let (segments, _) = collect(&mut session, 0);
    let (seg_conn, _) = segments
        .iter()
        .find(|(_, u)| u == TS_SEG0_URL)
        .cloned()
        .unwrap();
    let head = representation_headers(100_000, Some(ETAG));
    session.on_response(seg_conn, 200, &head, 1_000);
    session.on_data(seg_conn, &[0u8; 100], 2_000);
    let deadline = 2_000 + INACTIVITY_US;
    assert!(!collect(&mut session, deadline - 1).1.contains(&seg_conn));
    assert!(collect(&mut session, deadline).1.contains(&seg_conn));
}

#[test]
fn a_server_that_ignores_ranges_has_its_whole_body_used_once() {
    let scenario = plain(Origin::IgnoresRanges, 5_000);
    let drive = scenario.run(config());
    assert_eq!(drive.samples(), baseline_samples());
    let requests = target_requests(&drive, &scenario);
    assert_eq!(requests.len(), 2);
    assert!(requests[1].header("range").is_some());
}

#[test]
fn a_changed_validator_gets_a_200_and_the_new_whole_body() {
    let scenario = plain(Origin::NewValidator, 5_000);
    let drive = scenario.run(config());
    assert_eq!(drive.samples(), baseline_samples());
    assert_eq!(target_requests(&drive, &scenario).len(), 2);
}

#[test]
fn a_206_naming_the_wrong_range_falls_back_to_one_immediate_full_request() {
    let scenario = plain(Origin::WrongContentRange, 5_000);
    let drive = scenario.run(config());
    assert_eq!(drive.samples(), baseline_samples());
    let requests = target_requests(&drive, &scenario);
    assert_eq!(requests.len(), 3);
    assert!(requests[1].header("range").is_some());
    assert_eq!(requests[2].header("range"), None);
    assert_eq!(requests[2].at_us, requests[1].at_us, "no second backoff");
    let attempts = drive
        .statuses()
        .into_iter()
        .filter(|s| matches!(s, Status::Reconnecting { .. }))
        .count();
    assert_eq!(attempts, 1);
}

#[test]
fn a_416_to_the_resume_falls_back_to_one_immediate_full_request() {
    let scenario = plain(Origin::Unsatisfiable, 5_000);
    let drive = scenario.run(config());
    assert_eq!(drive.samples(), baseline_samples());
    let requests = target_requests(&drive, &scenario);
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].header("range"), None);
}

#[test]
fn without_a_strong_validator_the_whole_resource_is_requested_again() {
    for origin in [Origin::NoValidator, Origin::WeakValidator] {
        let scenario = plain(origin, 5_000);
        let drive = scenario.run(config());
        assert_eq!(drive.samples(), baseline_samples(), "{origin:?}");
        let requests = target_requests(&drive, &scenario);
        assert_eq!(requests.len(), 2, "{origin:?}");
        assert_eq!(requests[1].header("range"), None, "{origin:?}");
        assert_eq!(drive.stalls(), vec![ResumeMode::Full], "{origin:?}");
    }
}

#[test]
fn a_byte_ranged_segment_resumes_inside_its_own_range() {
    let sent = 4_000;
    let (scenario, baseline) = byte_ranged(Origin::Honours, sent);
    let drive = scenario.run(config());
    assert_eq!(drive.samples(), baseline);
    let len0 = ts_segment(0).len();
    let requests = target_requests(&drive, &scenario);
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].byte_range,
        Some((0, u64::try_from(len0).unwrap()))
    );
    assert_eq!(
        requests[1].header("range"),
        Some(format!("bytes={sent}-{}", len0 - 1).as_str())
    );
    assert_eq!(requests[1].byte_range, None);
}

#[test]
fn a_byte_ranged_resume_answered_with_the_whole_file_is_cut_to_its_range() {
    let (scenario, baseline) = byte_ranged(Origin::IgnoresRanges, 4_000);
    let drive = scenario.run(config());
    assert_eq!(drive.samples(), baseline);
}

#[test]
fn every_origin_behaviour_at_every_stall_point_yields_the_origins_exact_bytes() {
    let want = baseline_samples();
    let total = ts_segment(0).len();
    let mut cells = 0;
    for &origin in ALL_ORIGINS {
        for sent in [1, total / 2, total - 1] {
            let scenario = plain(origin, sent);
            let drive = scenario.run(config());
            assert_eq!(drive.samples(), want, "{origin:?} stalled after {sent}");
            let expected_mode = if origin.resumes() {
                ResumeMode::Range
            } else {
                ResumeMode::Full
            };
            assert_eq!(drive.stalls(), vec![expected_mode], "{origin:?} {sent}");
            assert_eq!(
                target_requests(&drive, &scenario).len(),
                2 + origin.extra_requests(),
                "{origin:?} {sent}"
            );
            cells += 1;
        }
    }
    assert_eq!(cells, ALL_ORIGINS.len() * 3);
}

#[test]
fn a_trickle_that_never_pauses_for_the_timeout_is_never_closed() {
    let mut drive = Drive::new(config());
    drive.collect();
    let playlist = drive.pending.pop().unwrap();
    drive.deliver(playlist.conn, Serve::Whole(200, Vec::new(), ts_playlist()));
    drive.collect();
    let neighbour = drive
        .log
        .iter()
        .find(|r| r.url == TS_SEG1_URL)
        .cloned()
        .unwrap();
    drive.deliver(neighbour.conn, Serve::Whole(200, Vec::new(), ts_segment(1)));
    let seg = drive
        .log
        .iter()
        .find(|r| r.url == TS_SEG0_URL)
        .cloned()
        .unwrap();
    let body = ts_segment(0);
    drive.session.on_response(
        seg.conn,
        200,
        &representation_headers(body.len(), Some(ETAG)),
        0,
    );
    let chunks = 12;
    let step = body.len().div_ceil(chunks);
    let gap = INACTIVITY_US - MS;
    for (i, piece) in body.chunks(step).enumerate() {
        drive.now = u64::try_from(i + 1).unwrap() * gap;
        drive.collect();
        assert!(
            drive.closed_at(seg.conn).is_none(),
            "closed while bytes kept arriving, at {} s",
            drive.now / SEC
        );
        drive.session.on_data(seg.conn, piece, drive.now);
    }
    drive.session.on_closed(seg.conn, None, drive.now);
    drive.collect();
    assert!(drive.stalls().is_empty());
    assert!(drive.closed_at(seg.conn).is_none());
    assert!(
        drive.now > 3 * INACTIVITY_US,
        "the transfer outlasted the timeout"
    );
}

#[test]
fn a_stalled_request_leaves_its_parallel_request_alone_and_keeps_the_inflight_cap() {
    let mut config = config();
    config.max_inflight_segments = 1;
    let scenario = plain(Origin::Honours, 5_000);
    let mut drive = Drive::new(config);
    let mut outstanding: BTreeSet<u32> = BTreeSet::new();
    let mut peak = 0;
    let mut nth_target = 0usize;
    loop {
        drive.collect();
        for r in &drive.pending {
            if r.url != TS_PLAYLIST_URL {
                outstanding.insert(r.conn);
            }
        }
        for (conn, _) in &drive.closes {
            outstanding.remove(conn);
        }
        peak = peak.max(outstanding.len());
        if drive.finished() {
            break;
        }
        if let Some(req) = drive.pending.pop() {
            let serve = scenario.answer(&req, nth_target);
            if req.url != TS_PLAYLIST_URL && (scenario.target)(&req) {
                nth_target += 1;
            }
            let completes = matches!(serve, Serve::Whole(..));
            drive.deliver(req.conn, serve);
            if completes {
                outstanding.remove(&req.conn);
            }
            continue;
        }
        let Some(next) = drive.wakes.range(drive.now + 1..).next().copied() else {
            break;
        };
        drive.wakes.remove(&next);
        drive.now = next;
    }
    assert_eq!(peak, 1, "never more than max_inflight_segments outstanding");
    assert_eq!(drive.samples(), baseline_samples());
    let seg1 = drive.log.iter().filter(|r| r.url == TS_SEG1_URL).count();
    let seg0 = drive.log.iter().filter(|r| r.url == TS_SEG0_URL).count();
    assert_eq!((seg0, seg1), (2, 1));
    let seg0_resume_at = drive
        .log
        .iter()
        .filter(|r| r.url == TS_SEG0_URL)
        .nth(1)
        .unwrap()
        .at_us;
    let seg1_at = drive
        .log
        .iter()
        .find(|r| r.url == TS_SEG1_URL)
        .unwrap()
        .at_us;
    assert!(
        seg1_at >= seg0_resume_at,
        "segment 1 waited for segment 0's slot"
    );
}

#[test]
fn only_the_stalled_connection_is_closed_while_a_parallel_segment_completes() {
    let scenario = plain(Origin::Honours, 5_000);
    let drive = scenario.run(config());
    let seg1_conns: Vec<u32> = drive
        .log
        .iter()
        .filter(|r| r.url == TS_SEG1_URL)
        .map(|r| r.conn)
        .collect();
    let stalled = target_requests(&drive, &scenario)[0].conn;
    assert_eq!(
        drive.closes.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
        vec![stalled]
    );
    assert!(seg1_conns.iter().all(|c| *c != stalled));
}

#[test]
fn a_segment_that_stalls_every_time_exhausts_the_retry_budget() {
    let mut config = config();
    config.max_segment_failures = 2;
    let mut drive = Drive::new(config);
    let file = ts_segment(0);
    let playlist = ts_playlist();
    drive.run(&mut |req| {
        if req.url == TS_PLAYLIST_URL {
            return Serve::Whole(200, Vec::new(), playlist.clone());
        }
        if req.url == TS_SEG0_URL {
            return Serve::Stall(
                200,
                representation_headers(file.len(), Some(ETAG)),
                file[..100].to_vec(),
            );
        }
        Serve::Whole(200, Vec::new(), ts_segment(1))
    });
    assert!(drive.events.iter().any(|e| matches!(
        e,
        Event::Error {
            kind: ErrorKind::SegmentFetchFailed { status: None },
            fatal: true
        }
    )));
    assert_eq!(drive.log.iter().filter(|r| r.url == TS_SEG0_URL).count(), 2);
    assert_eq!(drive.stalls().len(), 2);
}

#[test]
fn a_playlist_is_exempt_and_only_the_request_timeout_bounds_it() {
    let mut config = config();
    config.request_timeout_us = 20 * SEC;
    let mut session = Session::open(TS_PLAYLIST_URL, config, 0).unwrap();
    let mut conn = None;
    let mut closed = false;
    for now in [0, INACTIVITY_US, 2 * INACTIVITY_US, 20 * SEC - 1, 20 * SEC] {
        while let Some(action) = session.poll(now) {
            match action {
                Action::HttpRequest { conn: c, .. } => conn = Some(c),
                Action::Close { conn: c } => {
                    assert_eq!(Some(c), conn);
                    assert_eq!(now, 20 * SEC, "closed by the request timeout only");
                    closed = true;
                }
                _ => {}
            }
        }
    }
    assert!(closed);
}

#[test]
fn a_stalled_init_section_is_closed_and_refetched_whole_when_it_never_answered() {
    let playlist = support::hls_host::cmaf_playlist_text().into_bytes();
    let init = support::hls_host::cmaf_init();
    let mut drive = Drive::new(config());
    let mut init_requests = 0;
    drive.run(&mut |req| {
        if req.url == TS_PLAYLIST_URL {
            return Serve::Whole(200, Vec::new(), playlist.clone());
        }
        if req.url.ends_with("init.mp4") {
            init_requests += 1;
            if init_requests == 1 {
                return Serve::Stall(
                    200,
                    representation_headers(init.len(), Some(ETAG)),
                    init[..10].to_vec(),
                );
            }
            return Serve::Whole(
                200,
                representation_headers(init.len(), Some(ETAG)),
                init.clone(),
            );
        }
        let n: usize = req
            .url
            .trim_end_matches(".m4s")
            .rsplit("index")
            .next()
            .unwrap()
            .parse()
            .unwrap();
        Serve::Whole(200, Vec::new(), support::hls_host::cmaf_segment(n))
    });
    assert_eq!(drive.stalls(), vec![ResumeMode::Range]);
    let init_reqs: Vec<&Req> = drive
        .log
        .iter()
        .filter(|r| r.url.ends_with("init.mp4"))
        .collect();
    assert_eq!(init_reqs.len(), 2);
    assert_eq!(
        init_reqs[1].header("range"),
        Some(format!("bytes=10-{}", init.len() - 1).as_str())
    );
    assert!(!drive.samples().is_empty());
}
