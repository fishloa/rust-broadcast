//! The HTTP layer of a DASH `Session`: credentials, cookies and extra headers on every request
//! class (MPD, `UTCTiming`, init, index, segment) scoped to the session origin, redirects, `Range`,
//! `Retry-After`, timeouts and the credential wait.

#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_sim.rs"]
mod dash_sim;

use broadcast_client::{Config, Credentials, ErrorKind, Event, Status};
use dash_common::loopback::{Origin, Sig, oracle};
use dash_common::synth::{Addressing, MPD_URL, Synth};
use dash_common::world::{MICROS, World};
use dash_sim::{Answer, Backend, DashSim, Logged, SEC, StaticBackend, WorldBackend};

const HOST: &str = "http://dash.test";
const MANIFEST: &str = "manifest.mpd";
const SIDX: &str = "ondemand-sidx/ondemand.mpd";
const AST_US: i64 = 1_700_000_000 * MICROS;
const UTC_URL: &str = "http://utc.test/xsdate";

fn url_of(rel: &str) -> String {
    format!("{HOST}/{rel}")
}

fn cdn_origin() -> Origin {
    Origin::new().mount("cdn.test", "")
}

fn with_cdn(rel: &str, config: Config) -> DashSim {
    let text = test_fixtures::read_to_string(&format!("dash/{rel}")).replacen(
        "<Period",
        "<BaseURL>http://cdn.test/</BaseURL><Period",
        1,
    );
    let url = url_of(rel);
    DashSim::new(
        &url,
        config,
        World::new(AST_US),
        Box::new(StaticBackend(cdn_origin().doc(&url, text.into_bytes()))),
    )
}

fn secured() -> Config {
    let mut config = Config::default();
    config.extra_headers = vec![("X-Probe".into(), "1".into())];
    config.initial_cookies = vec![("sid".into(), "7".into())];
    config
}

fn carries_all(l: &Logged) -> bool {
    l.header("authorization") == Some("Bearer tok")
        && l.header("cookie").is_some_and(|c| c.contains("sid=7"))
        && l.header("x-probe") == Some("1")
}

fn carries_none(l: &Logged) -> bool {
    l.header("authorization").is_none()
        && l.header("cookie").is_none()
        && l.header("x-probe").is_none()
}

fn live_with_utc(url: &'static str, config: Config) -> DashSim {
    let synth = Synth::new(AST_US, Addressing::Number).with_utc("http-xsdate", url);
    DashSim::new(
        MPD_URL,
        config,
        World::new(AST_US + 41 * MICROS + 120_000),
        Box::new(WorldBackend(synth)),
    )
}

struct Rewrite<B: Backend> {
    inner: B,
    from: &'static str,
    to: &'static str,
}

impl<B: Backend> Backend for Rewrite<B> {
    fn serve(&mut self, at: i64, req: &Logged) -> Answer {
        let mut r = req.clone();
        if let Some(rest) = req.url.strip_prefix(self.from) {
            r.url = format!("{}{rest}", self.to);
        }
        self.inner.serve(at, &r)
    }
}

fn whole(rel: &str) -> Vec<Sig> {
    oracle(&[rel]).remove(0)
}

#[test]
fn credentials_cookies_and_headers_reach_every_request_class_on_the_session_origin() {
    let mut sim = DashSim::static_origin(&url_of(SIDX), secured());
    sim.session
        .set_credentials(Credentials::bearer("tok"))
        .unwrap();
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    let mpd = sim.log.iter().filter(|l| l.is_mpd()).count();
    let ranged = sim.log.iter().filter(|l| l.byte_range.is_some()).count();
    assert_eq!(
        (mpd, ranged),
        (1, 16),
        "one MPD; init + sidx + 6 subsegments for each of two Representations"
    );
    assert!(
        sim.log.iter().all(carries_all),
        "{:?}",
        sim.log.iter().find(|l| !carries_all(l))
    );
}

#[test]
fn a_utc_server_and_a_cdn_on_other_origins_receive_nothing_the_session_origin_does() {
    let mut sim = live_with_utc(UTC_URL, secured());
    sim.session
        .set_credentials(Credentials::bearer("tok"))
        .unwrap();
    sim.run_for(15 * SEC);
    let utc: Vec<&Logged> = sim
        .log
        .iter()
        .filter(|l| l.url.starts_with("http://utc.test/"))
        .collect();
    let own: Vec<&Logged> = sim
        .log
        .iter()
        .filter(|l| l.url.starts_with("http://dash.test/"))
        .collect();
    assert!(!utc.is_empty() && !own.is_empty());
    assert!(utc.iter().all(|l| carries_none(l)), "{utc:?}");
    assert!(own.iter().all(|l| carries_all(l)));
    assert!(sim.fatal().is_none());

    let mut sim = with_cdn(MANIFEST, secured());
    sim.session
        .set_credentials(Credentials::bearer("tok"))
        .unwrap();
    sim.run_to_terminal(60 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    let cdn: Vec<&Logged> = sim
        .log
        .iter()
        .filter(|l| l.url.starts_with("http://cdn.test/"))
        .collect();
    assert!(cdn.len() >= 9);
    assert!(cdn.iter().all(|l| carries_none(l)));
    assert!(sim.log.iter().filter(|l| l.is_mpd()).all(carries_all));
}

#[test]
fn a_utc_server_on_the_session_origin_gets_the_credentials_too() {
    let synth = Synth::new(AST_US, Addressing::Number)
        .with_utc("http-xsdate", "http://dash.test/live/utc/xsdate");
    let backend = Rewrite {
        inner: WorldBackend(synth),
        from: "http://dash.test/live/utc/",
        to: "http://utc.test/",
    };
    let mut sim = DashSim::new(
        MPD_URL,
        secured(),
        World::new(AST_US + 41 * MICROS + 120_000),
        Box::new(backend),
    );
    sim.session
        .set_credentials(Credentials::bearer("tok"))
        .unwrap();
    sim.run_for(15 * SEC);
    let utc: Vec<&Logged> = sim
        .log
        .iter()
        .filter(|l| l.url.contains("/live/utc/"))
        .collect();
    assert!(!utc.is_empty());
    assert!(utc.iter().all(|l| carries_all(l)), "{utc:?}");
}

#[test]
fn a_401_waits_for_credentials_and_replays_the_parked_request() {
    struct Gate(StaticBackend);
    impl Backend for Gate {
        fn serve(&mut self, at: i64, req: &Logged) -> Answer {
            if req.header("authorization").is_none() {
                return Answer::status_with(
                    401,
                    vec![("WWW-Authenticate".into(), "Basic realm=\"r\"".into())],
                );
            }
            self.0.serve(at, req)
        }
    }
    let mut sim = DashSim::new(
        &url_of(MANIFEST),
        Config::default(),
        World::new(AST_US),
        Box::new(Gate(StaticBackend(Origin::new()))),
    );
    sim.run_for(30 * SEC);
    assert!(
        sim.events
            .iter()
            .any(|e| matches!(e, Event::AuthRequired { .. })),
        "{:?}",
        sim.events
    );
    assert!(sim.statuses().contains(&Status::WaitingForCredentials));
    let waiting = sim.log.len();
    sim.run_for(60 * SEC);
    assert_eq!(sim.log.len(), waiting, "no request is made while waiting");
    sim.session
        .set_credentials(Credentials::new("u", "p"))
        .unwrap();
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    let mpd = sim.log.iter().filter(|l| l.is_mpd()).collect::<Vec<_>>();
    assert!(
        mpd.last()
            .unwrap()
            .header("authorization")
            .unwrap()
            .starts_with("Basic ")
    );
}

#[test]
fn a_403_is_reported_and_the_request_is_retried() {
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), Config::default());
    let mut first = true;
    sim.fault = Some(Box::new(move |req: &Logged, _| {
        if req.url.ends_with("chunk-stream0-00002.m4s") && first {
            first = false;
            return Some(Answer::status(403));
        }
        None
    }));
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended());
    assert_eq!(
        sim.count(|e| matches!(e, Event::Forbidden { status: 403 })),
        1
    );
    assert_eq!(sim.requests_to("chunk-stream0-00002").len(), 2);
}

#[test]
fn an_mpd_redirect_moves_the_base_for_every_relative_reference() {
    let old = url_of("old/manifest.mpd");
    let mut sim = DashSim::new(
        &old,
        Config::default(),
        World::new(AST_US),
        Box::new(StaticBackend(Origin::new())),
    );
    sim.fault = Some(Box::new(|req: &Logged, _| {
        (req.url == "http://dash.test/old/manifest.mpd")
            .then(|| Answer::Redirect("http://dash.test/manifest.mpd".into()))
    }));
    sim.run_to_terminal(60 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    assert!(
        sim.log
            .iter()
            .all(|l| !l.url.contains("/old/chunk") && !l.url.contains("/old/init")),
        "segments resolve against the redirect target"
    );
    assert_eq!(sim.log.iter().filter(|l| l.is_mpd()).count(), 2);
    let first = sim.log.iter().find(|l| l.is_mpd()).unwrap().conn;
    assert!(
        sim.closes.contains(&first),
        "the redirected connection is closed"
    );
}

#[test]
fn a_segment_redirect_to_a_cdn_is_followed_without_credentials() {
    let mut config = secured();
    config.max_redirects = 3;
    let origin = cdn_origin();
    let mut sim = DashSim::new(
        &url_of(MANIFEST),
        config,
        World::new(AST_US),
        Box::new(StaticBackend(origin)),
    );
    sim.session
        .set_credentials(Credentials::bearer("tok"))
        .unwrap();
    sim.fault = Some(Box::new(|req: &Logged, _| {
        (req.url == "http://dash.test/chunk-stream0-00002.m4s")
            .then(|| Answer::Redirect("http://cdn.test/chunk-stream0-00002.m4s".into()))
    }));
    sim.run_to_terminal(60 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    let hop = sim.requests_to("http://cdn.test/chunk-stream0-00002.m4s");
    assert_eq!(hop.len(), 1);
    assert!(carries_none(hop[0]));
    let origin_leg = sim.requests_to("http://dash.test/chunk-stream0-00002.m4s");
    assert!(carries_all(origin_leg[0]));
}

#[test]
fn a_redirect_loop_is_bounded_per_attempt_and_ends_as_a_failed_segment() {
    let mut config = Config::default();
    config.max_redirects = 2;
    config.max_segment_failures = 2;
    let mut sim = DashSim::new(
        &url_of(MANIFEST),
        config,
        World::new(AST_US),
        Box::new(StaticBackend(Origin::new())),
    );
    sim.fault = Some(Box::new(|req: &Logged, _| {
        req.url
            .ends_with("chunk-stream0-00001.m4s")
            .then(|| Answer::Redirect(req.url.clone()))
    }));
    sim.run_to_terminal(120 * SEC);
    assert_eq!(
        sim.fatal(),
        Some(ErrorKind::SegmentFetchFailed { status: None })
    );
    let hops = sim.requests_to("chunk-stream0-00001").len();
    assert_eq!(
        hops,
        2 * (1 + 2),
        "two attempts of one request and two redirects each"
    );
}

#[test]
fn no_redirects_allowed_fails_an_mpd_redirect_and_retries_it() {
    let mut config = Config::default();
    config.max_redirects = 0;
    let mut sim = DashSim::new(
        &url_of("old/manifest.mpd"),
        config,
        World::new(AST_US),
        Box::new(StaticBackend(Origin::new())),
    );
    sim.fault = Some(Box::new(|req: &Logged, _| {
        req.is_mpd()
            .then(|| Answer::Redirect("http://dash.test/manifest.mpd".into()))
    }));
    sim.run_for(30 * SEC);
    assert_eq!(sim.count(|e| matches!(e, Event::Sample { .. })), 0);
    assert!(sim.log.iter().filter(|l| l.is_mpd()).count() >= 3);
    assert!(sim.fatal().is_none());
}

#[test]
fn a_hung_segment_is_closed_at_the_segment_timeout_and_requested_again() {
    let mut config = Config::default();
    config.segment_timeout_us = 3 * SEC;
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), config);
    let mut hung = false;
    sim.fault = Some(Box::new(move |req: &Logged, _| {
        if req.url.ends_with("chunk-stream1-00002.m4s") && !hung {
            hung = true;
            return Some(Answer::Hang);
        }
        None
    }));
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    let reqs = sim.requests_to("chunk-stream1-00002");
    assert_eq!(reqs.len(), 2);
    assert!(
        sim.closes.contains(&reqs[0].conn),
        "the hung connection was closed"
    );
    assert!(
        reqs[1].send_elapsed_us >= reqs[0].send_elapsed_us + 3 * SEC,
        "not before the timeout"
    );
}

#[test]
fn a_whole_file_answer_to_a_ranged_request_is_sliced_and_a_wrong_range_is_a_failure() {
    let mut sim = DashSim::static_origin(&url_of(SIDX), Config::default());
    sim.fault = Some(Box::new(|req: &Logged, _| {
        req.byte_range.map(|_| {
            let file = if req.url.contains("/a_") {
                "a_dashinit.mp4"
            } else {
                "v_dashinit.mp4"
            };
            Answer::ok(test_fixtures::read(&format!("dash/ondemand-sidx/{file}")))
        })
    }));
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    dash_sim::assert_samples_equal_oracle(
        &sim,
        &[
            whole("ondemand-sidx/v_dashinit.mp4"),
            whole("ondemand-sidx/a_dashinit.mp4"),
        ],
    );

    let mut sim = DashSim::static_origin(&url_of(SIDX), Config::default());
    let mut once = true;
    sim.fault = Some(Box::new(move |req: &Logged, _| {
        if req.byte_range.is_some() && once {
            once = false;
            return Some(Answer::Full {
                status: 206,
                headers: vec![("Content-Range".into(), "bytes 0-1/*".into())],
                body: vec![0, 1],
            });
        }
        None
    }));
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    assert!(
        sim.statuses()
            .iter()
            .any(|s| matches!(s, Status::Reconnecting { attempt: 1 }))
    );
}

#[test]
fn retry_after_sets_the_wait_for_a_429_and_cookies_from_the_mpd_are_kept() {
    let mut sim = DashSim::static_origin(&url_of(MANIFEST), Config::default());
    let mut first = true;
    sim.fault = Some(Box::new(move |req: &Logged, _| {
        if req.is_mpd() {
            let text = test_fixtures::read(&format!("dash/{MANIFEST}"));
            return Some(Answer::ok(text).with_header("Set-Cookie", "sid=abc; Path=/"));
        }
        if req.url.ends_with("chunk-stream0-00003.m4s") && first {
            first = false;
            return Some(Answer::status_with(
                429,
                vec![("Retry-After".into(), "9".into())],
            ));
        }
        None
    }));
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    let reqs = sim.requests_to("chunk-stream0-00003");
    assert_eq!(reqs.len(), 2);
    assert_eq!((reqs[1].send_elapsed_us - reqs[0].send_elapsed_us) / SEC, 9);
    assert!(
        sim.log
            .iter()
            .skip(1)
            .all(|l| l.header("cookie") == Some("sid=abc")),
        "the MPD's cookie rides on every later request"
    );
    assert!(
        sim.session
            .cookies()
            .iter()
            .any(|(k, v)| k == "sid" && v == "abc")
    );
}
