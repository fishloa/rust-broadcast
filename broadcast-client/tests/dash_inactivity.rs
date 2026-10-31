//! Per-request inactivity timeout of the DASH source with `Range` resume (issue #1613): a media
//! request that goes silent after a prefix is closed at exactly `segment_inactivity_timeout_us`
//! and asked for again on the retry schedule, resuming with `Range`/`If-Range` (RFC 9110 14.2,
//! 13.1.5) when the origin allows, over the real DASH fixtures.

#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_sim.rs"]
mod dash_sim;

use broadcast_client::{Config, ErrorKind, Event, ResumeMode, Status};
use dash_common::loopback::Origin;
use dash_common::world::World;
use dash_sim::{Answer, Backend, DashSim, Logged, SEC, SIM_ETAG, StaticBackend, stall_after};

const MPD: &str = "http://dash.test/manifest.mpd";
const SEGMENT: &str = "http://dash.test/chunk-stream0-00002.m4s";
const SIDX_MPD: &str = "http://dash.test/ondemand-sidx/ondemand.mpd";
const SIDX_VIDEO: &str = "http://dash.test/ondemand-sidx/v_dashinit.mp4";
const AST_US: i64 = 1_700_000_000 * 1_000_000;
const INACTIVITY_US: u64 = 4 * SEC;
const FIRST_BACKOFF_US: u64 = 2 * SEC;
const HORIZON_US: u64 = 900 * SEC;
const MEDIA_RANGE_MIN_LEN: u64 = 1_000;

fn config() -> Config {
    let mut config = Config::default();
    config.segment_inactivity_timeout_us = INACTIVITY_US;
    config.request_timeout_us = 30 * SEC;
    config.segment_timeout_us = 60 * SEC;
    config
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Resumed {
    Honoured,
    NewValidator,
    ContentRangeOffByOne,
}

struct Stall {
    inner: Box<dyn Backend>,
    url: String,
    from: Option<u64>,
    fraction: usize,
    resumed: Resumed,
    permanent: bool,
    stalled: bool,
}

impl Stall {
    fn boxed(
        url: &str,
        from: Option<u64>,
        resumed: Resumed,
        permanent: bool,
        origin: Box<dyn Backend>,
    ) -> Box<dyn Backend> {
        Box::new(Self {
            inner: origin,
            url: url.to_owned(),
            from,
            fraction: 2,
            resumed,
            permanent,
            stalled: false,
        })
    }

    fn mine(&self, req: &Logged) -> bool {
        req.url == self.url
            && self
                .from
                .is_none_or(|f| req.byte_range.is_some_and(|(o, _)| o == f))
    }
}

impl Backend for Stall {
    fn serve(&mut self, at: i64, req: &Logged) -> Answer {
        let good = self.inner.serve(at, req);
        if !self.mine(req) {
            return good;
        }
        let resume = req.header("range").is_some();
        if !self.stalled || self.permanent {
            self.stalled = true;
            let Answer::Full { body, .. } = &good else {
                return good;
            };
            let sent = body.len() / self.fraction;
            return stall_after(&good, sent).expect("a 200 or 206 to stall");
        }
        match (resume, self.resumed, good) {
            (true, Resumed::NewValidator, Answer::Full { status, mut headers, body }) => {
                headers.push(("ETag".into(), "\"sim-v2\"".into()));
                Answer::Full { status, headers, body }
            }
            (true, Resumed::ContentRangeOffByOne, Answer::Full { body, .. }) => {
                let range = req.header("range").unwrap();
                let first: u64 = range[6..].split('-').next().unwrap().parse().unwrap();
                let total = body.len() as u64;
                Answer::Full {
                    status: 206,
                    headers: vec![
                        (
                            "Content-Range".into(),
                            format!("bytes {}-{}/{total}", first + 1, total - 1),
                        ),
                        ("ETag".into(), SIM_ETAG.into()),
                    ],
                    body: body[usize::try_from(first + 1).unwrap()..].to_vec(),
                }
            }
            (_, _, good) => good,
        }
    }
}

fn sim(mpd: &str, backend: Box<dyn Backend>) -> DashSim {
    DashSim::new(mpd, config(), World::new(AST_US), backend)
}

fn static_origin() -> Box<dyn Backend> {
    Box::new(StaticBackend(Origin::new()))
}

fn run(mut sim: DashSim) -> DashSim {
    sim.run_to_terminal(HORIZON_US);
    sim
}

fn baseline(mpd: &str) -> DashSim {
    run(sim(mpd, static_origin()))
}

fn assert_same_media(got: &DashSim, want: &DashSim) {
    let (got, want) = (got.samples(), want.samples());
    assert_eq!(got.keys().collect::<Vec<_>>(), want.keys().collect::<Vec<_>>());
    for (track, want) in &want {
        let have = &got[track];
        assert_eq!(
            dash_sim::sigs_of(have),
            dash_sim::sigs_of(want),
            "track {track}: the joined body differs from the origin's"
        );
        assert_eq!(have.len(), want.len(), "track {track}");
    }
}

fn header<'l>(req: &'l Logged, name: &str) -> Option<&'l str> {
    req.header(name)
}

fn media_requests<'s>(sim: &'s DashSim, url: &str) -> Vec<&'s Logged> {
    sim.log.iter().filter(|l| l.url == url).collect()
}

fn stall_modes(sim: &DashSim) -> Vec<ResumeMode> {
    sim.statuses()
        .into_iter()
        .filter_map(|s| match s {
            Status::RequestStalled { resume } => Some(resume),
            _ => None,
        })
        .collect()
}

#[test]
fn a_silent_segment_closes_at_exactly_the_inactivity_timeout_and_resumes_with_a_range() {
    let want = baseline(MPD);
    let got = run(sim(
        MPD,
        Stall::boxed(SEGMENT, None, Resumed::Honoured, false, static_origin()),
    ));
    assert_same_media(&got, &want);
    assert!(got.ended());

    let requests = media_requests(&got, SEGMENT);
    assert_eq!(requests.len(), 2, "one stalled request, one resume");
    let (first, second) = (requests[0], requests[1]);
    let last_byte_at = first.send_elapsed_us + got.rtt_us;
    let closed_at = got
        .close_log
        .iter()
        .find(|(conn, _)| *conn == first.conn)
        .map(|(_, at)| *at)
        .expect("the stalled request was closed");
    assert_eq!(closed_at, last_byte_at + INACTIVITY_US);

    assert_eq!(
        second.send_elapsed_us,
        closed_at + FIRST_BACKOFF_US,
        "the resume waits for the retry backoff"
    );
    let range = header(second, "range").expect("a Range header");
    assert!(range.starts_with("bytes="), "{range}");
    assert_eq!(header(second, "if-range"), Some(SIM_ETAG));
    assert_eq!(second.byte_range, None);
    assert_eq!(second.status, Some(206));
    assert_eq!(header(first, "range"), None);

    assert_eq!(stall_modes(&got), vec![ResumeMode::Range]);
    assert!(
        got.statuses()
            .contains(&Status::Reconnecting { attempt: 1 }),
        "the stall counts as a failed attempt: {:?}",
        got.statuses()
    );
}

#[test]
fn the_resume_range_starts_exactly_after_the_received_prefix() {
    let want = baseline(MPD);
    let whole = Origin::new()
        .serve(&dash_common::loopback::Req {
            url: SEGMENT.to_owned(),
            range: None,
        })
        .unwrap()
        .body
        .len();
    let got = run(sim(
        MPD,
        Stall::boxed(SEGMENT, None, Resumed::Honoured, false, static_origin()),
    ));
    assert_same_media(&got, &want);
    let second = media_requests(&got, SEGMENT)[1];
    let sent = whole / 2;
    assert_eq!(
        header(second, "range"),
        Some(format!("bytes={sent}-{}", whole - 1).as_str())
    );
}

#[test]
fn an_origin_that_ignores_ranges_has_its_whole_body_used_once() {
    let want = baseline(MPD);
    let mut faulty = sim(
        MPD,
        Stall::boxed(SEGMENT, None, Resumed::Honoured, false, static_origin()),
    );
    faulty.honour_ranges = false;
    let got = run(faulty);
    assert_same_media(&got, &want);
    let requests = media_requests(&got, SEGMENT);
    assert_eq!(requests.len(), 2);
    assert!(header(requests[1], "range").is_some(), "the resume was asked for");
    assert_eq!(requests[1].status, Some(200));
    assert_eq!(stall_modes(&got), vec![ResumeMode::Range]);
}

#[test]
fn a_changed_validator_gets_the_whole_new_body_not_a_splice() {
    let want = baseline(MPD);
    let got = run(sim(
        MPD,
        Stall::boxed(SEGMENT, None, Resumed::NewValidator, false, static_origin()),
    ));
    assert_same_media(&got, &want);
    let requests = media_requests(&got, SEGMENT);
    assert_eq!(requests.len(), 2);
    assert_eq!(header(requests[1], "if-range"), Some(SIM_ETAG));
    assert_eq!(requests[1].status, Some(200), "If-Range failed, whole body");
}

#[test]
fn a_206_that_does_not_continue_the_prefix_falls_back_to_one_full_request() {
    let want = baseline(MPD);
    let got = run(sim(
        MPD,
        Stall::boxed(
            SEGMENT,
            None,
            Resumed::ContentRangeOffByOne,
            false,
            static_origin(),
        ),
    ));
    assert_same_media(&got, &want);
    let requests = media_requests(&got, SEGMENT);
    assert_eq!(requests.len(), 3, "stalled, bad resume, full re-fetch");
    assert!(header(requests[1], "range").is_some());
    assert_eq!(header(requests[2], "range"), None);
    assert_eq!(
        requests[2].send_elapsed_us - requests[1].send_elapsed_us,
        got.rtt_us,
        "the fallback is immediate, not another backoff"
    );
    let attempts = got
        .statuses()
        .into_iter()
        .filter(|s| matches!(s, Status::Reconnecting { .. }))
        .count();
    assert_eq!(attempts, 1, "only the stall counted as a failure");
}

#[test]
fn a_ranged_media_request_resumes_inside_its_own_range() {
    let want = baseline(SIDX_MPD);
    let (offset, len) = want
        .log
        .iter()
        .filter(|l| l.url == SIDX_VIDEO)
        .filter_map(|l| l.byte_range)
        .find(|(_, len)| *len > MEDIA_RANGE_MIN_LEN)
        .expect("a ranged media request");
    let got = run(sim(
        SIDX_MPD,
        Stall::boxed(
            SIDX_VIDEO,
            Some(offset),
            Resumed::Honoured,
            false,
            static_origin(),
        ),
    ));
    assert_same_media(&got, &want);
    let resumes: Vec<&Logged> = got
        .log
        .iter()
        .filter(|l| l.url == SIDX_VIDEO && header(l, "range").is_some())
        .collect();
    assert_eq!(resumes.len(), 1);
    let sent = len / 2;
    assert_eq!(
        header(resumes[0], "range"),
        Some(format!("bytes={}-{}", offset + sent, offset + len - 1).as_str())
    );
    assert_eq!(resumes[0].byte_range, None);
    assert_eq!(resumes[0].status, Some(206));
}

#[test]
fn a_stalled_request_never_resumes_a_neighbour() {
    let want = baseline(MPD);
    let got = run(sim(
        MPD,
        Stall::boxed(SEGMENT, None, Resumed::Honoured, false, static_origin()),
    ));
    let other_conns: Vec<u32> = got
        .log
        .iter()
        .filter(|l| l.url != SEGMENT && !l.url.ends_with(".mpd"))
        .map(|l| l.conn)
        .collect();
    let stalled_conn = media_requests(&got, SEGMENT)[0].conn;
    assert!(
        got.closes.iter().all(|c| !other_conns.contains(c)),
        "a neighbouring request is never closed for the stall"
    );
    assert_eq!(got.closes.iter().filter(|c| **c == stalled_conn).count(), 1);
    assert_same_media(&got, &want);
}

#[test]
fn a_segment_that_stalls_every_time_exhausts_the_retry_budget() {
    let mut c = config();
    c.max_segment_failures = 2;
    let faulty = DashSim::new(
        MPD,
        c,
        World::new(AST_US),
        Stall::boxed(SEGMENT, None, Resumed::Honoured, true, static_origin()),
    );
    let got = run(faulty);
    assert_eq!(
        got.fatal(),
        Some(ErrorKind::SegmentFetchFailed { status: None })
    );
    let requests = media_requests(&got, SEGMENT).len();
    assert_eq!(requests, 2, "one request per allowed failure");
    assert_eq!(stall_modes(&got).len(), 2);
}

#[test]
fn a_silent_connection_without_a_response_head_is_refetched_whole() {
    let want = baseline(MPD);
    struct Hang(Box<dyn Backend>, bool);
    impl Backend for Hang {
        fn serve(&mut self, at: i64, req: &Logged) -> Answer {
            let good = self.0.serve(at, req);
            if req.url == SEGMENT && !self.1 {
                self.1 = true;
                return Answer::Hang;
            }
            good
        }
    }
    let got = run(sim(MPD, Box::new(Hang(static_origin(), false))));
    assert_same_media(&got, &want);
    let requests = media_requests(&got, SEGMENT);
    assert_eq!(requests.len(), 2);
    assert_eq!(header(requests[1], "range"), None);
    assert_eq!(stall_modes(&got), vec![ResumeMode::Full]);
    let closed_at = got
        .close_log
        .iter()
        .find(|(conn, _)| *conn == requests[0].conn)
        .map(|(_, at)| *at)
        .unwrap();
    assert_eq!(closed_at, requests[0].send_elapsed_us + INACTIVITY_US);
}

#[test]
fn nothing_changes_when_no_request_stalls() {
    let got = baseline(MPD);
    assert!(stall_modes(&got).is_empty());
    assert!(got.ended());
    assert!(!got.events.iter().any(|e| matches!(e, Event::Error { .. })));
    assert!(
        got.log.iter().all(|l| header(l, "range").is_none()),
        "no resume without a stall"
    );
}
