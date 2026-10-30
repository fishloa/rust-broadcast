//! A server that refuses with `429`/`503`: the `Retry-After` it names paces the next attempt of
//! every HTTP source, and `Event::ServerRefused` tells the host why.

#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_sim.rs"]
mod dash_sim;
mod support;

use broadcast_client::{Action, Config, Event, MAX_SERVER_REFUSED_REASON_BYTES, Session, Status};
use dash_sim::{Answer, DashSim, Logged};
use support::hls_host::{
    Host, Reply, SEC, TS_PLAYLIST_URL, TS_SEG0_URL, TS_SEG1_URL, hls_config, ts_playlist,
    ts_segment,
};

const TS_URL: &str = "http://h/live.ts";
const STEPS: usize = 2_000;
const REASON: &str = "descrambler busy: all 4 CAM slots in use";
const MAX_RETRY_AFTER_SECS: u64 = 60;
const BASE_DATE: &str = "Mon, 26 Oct 2026 08:39:04 GMT";
const PLUS_30S: &str = "Mon, 26 Oct 2026 08:39:34 GMT";
const PLUS_1H: &str = "Mon, 26 Oct 2026 09:39:04 GMT";
const DASH_MANIFEST: &str = "http://dash.test/manifest.mpd";

fn header(name: &str, value: &str) -> (String, String) {
    (name.to_string(), value.to_string())
}

fn refusal(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Reply {
    Reply::Full(
        status,
        body.to_vec(),
        headers.iter().map(|(k, v)| header(k, v)).collect(),
    )
}

fn busy(retry_after: &str) -> Reply {
    refusal(
        503,
        &[
            ("Retry-After", retry_after),
            ("Content-Type", "text/plain; charset=utf-8"),
        ],
        REASON.as_bytes(),
    )
}

fn ts_host(replies: Vec<Reply>) -> Host {
    let mut h = Host::new(TS_URL, Config::default());
    h.route(TS_URL, replies);
    h
}

fn request_times(h: &mut Host, count: usize) -> Vec<u64> {
    for _ in 0..STEPS {
        h.collect();
        if h.requests_to(TS_URL).len() >= count || !h.step() {
            break;
        }
    }
    h.collect();
    h.requests_to(TS_URL).iter().map(|s| s.at_us).collect()
}

fn gaps_secs(at: &[u64]) -> Vec<u64> {
    at.windows(2).map(|w| (w[1] - w[0]) / SEC).collect()
}

fn refused(events: &[Event]) -> Vec<(u16, Option<u64>, Option<String>)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ServerRefused {
                status,
                retry_after_us,
                reason,
            } => Some((*status, *retry_after_us, reason.clone())),
            _ => None,
        })
        .collect()
}

fn reason_of(content_type: Option<&str>, body: &[u8]) -> Option<String> {
    let headers: Vec<(&str, &str)> = content_type
        .map(|ct| ("Content-Type", ct))
        .into_iter()
        .collect();
    let mut h = ts_host(vec![refusal(503, &headers, body)]);
    request_times(&mut h, 2);
    let mut all = refused(&h.events);
    assert_eq!(all.len(), 1, "one refused attempt, one notice");
    all.remove(0).2
}

#[test]
fn ts_http_waits_the_retry_after_before_the_next_request() {
    let mut h = ts_host(vec![busy("30")]);
    let at = request_times(&mut h, 2);
    assert_eq!(at.len(), 2, "{at:?}");
    assert_eq!(
        at[1] - at[0],
        30 * SEC,
        "no request before 30 s, one at 30 s"
    );
}

#[test]
fn ts_http_retry_after_as_an_http_date_counts_from_the_response_date() {
    let mut h = ts_host(vec![refusal(
        503,
        &[("Date", BASE_DATE), ("Retry-After", PLUS_30S)],
        b"",
    )]);
    let at = request_times(&mut h, 2);
    assert_eq!(gaps_secs(&at), vec![30]);
}

#[test]
fn ts_http_retry_after_is_capped_at_sixty_seconds() {
    let mut h = ts_host(vec![busy("3600")]);
    let at = request_times(&mut h, 2);
    assert_eq!(gaps_secs(&at), vec![MAX_RETRY_AFTER_SECS]);
    let mut h = ts_host(vec![refusal(
        503,
        &[("Date", BASE_DATE), ("Retry-After", PLUS_1H)],
        b"",
    )]);
    let at = request_times(&mut h, 2);
    assert_eq!(gaps_secs(&at), vec![MAX_RETRY_AFTER_SECS]);
}

#[test]
fn ts_http_without_retry_after_keeps_the_normal_backoff() {
    let mut h = ts_host(vec![refusal(503, &[], b"")]);
    let at = request_times(&mut h, 4);
    assert_eq!(gaps_secs(&at), vec![2, 4, 8]);
}

#[test]
fn ts_http_ignores_retry_after_on_other_statuses() {
    for status in [500, 404, 502] {
        let mut h = ts_host(vec![refusal(status, &[("Retry-After", "30")], b"")]);
        let at = request_times(&mut h, 2);
        assert_eq!(gaps_secs(&at), vec![2], "status {status}");
        assert!(refused(&h.events).is_empty(), "status {status}");
    }
}

#[test]
fn ts_http_retry_after_does_not_outlive_its_response() {
    let mut h = ts_host(vec![busy("30"), refusal(503, &[], b"")]);
    let at = request_times(&mut h, 3);
    assert_eq!(gaps_secs(&at), vec![30, 4]);
}

#[test]
fn ts_http_a_short_retry_after_never_shortens_the_backoff() {
    let mut h = ts_host(vec![busy("1")]);
    let at = request_times(&mut h, 2);
    assert_eq!(gaps_secs(&at), vec![2]);
}

#[test]
fn ts_http_reports_server_refused_once_per_refused_attempt() {
    let mut h = ts_host(vec![busy("30")]);
    let at = request_times(&mut h, 3);
    assert_eq!(at.len(), 3);
    let notices = refused(&h.events);
    assert_eq!(notices.len(), 2, "two refused attempts completed");
    for (status, retry_after_us, reason) in notices {
        assert_eq!(status, 503);
        assert_eq!(retry_after_us, Some(30 * SEC));
        assert_eq!(reason.as_deref(), Some(REASON));
    }
    let reconnecting = h
        .events
        .iter()
        .filter(|e| matches!(e, Event::Status(Status::Reconnecting { .. })))
        .count();
    assert!(reconnecting >= 2);
}

#[test]
fn ts_http_server_refused_without_retry_after_carries_none() {
    let mut h = ts_host(vec![refusal(429, &[], b"")]);
    request_times(&mut h, 2);
    assert_eq!(refused(&h.events), vec![(429, None, None)]);
}

#[test]
fn the_reason_has_control_characters_stripped_and_is_trimmed() {
    let got = reason_of(Some("text/plain"), b"  bu\x00sy\x07\x1b\r\n\t slots \x7f ");
    assert_eq!(got.as_deref(), Some("busy slots"));
}

#[test]
fn the_reason_is_cut_on_a_char_boundary_at_the_cap() {
    let body = "\u{20ac}".repeat(100);
    assert!(body.len() > MAX_SERVER_REFUSED_REASON_BYTES);
    let got = reason_of(Some("text/plain; charset=UTF-8"), body.as_bytes()).unwrap();
    assert_eq!(got.len(), 255);
    assert!(got.chars().all(|c| c == '\u{20ac}'));
    let ascii = "x".repeat(1000);
    let got = reason_of(Some("text/plain"), ascii.as_bytes()).unwrap();
    assert_eq!(got.len(), MAX_SERVER_REFUSED_REASON_BYTES);
}

#[test]
fn the_reason_is_decoded_as_lossy_utf8() {
    let got = reason_of(Some("text/plain"), b"bad \xff byte");
    assert_eq!(got.as_deref(), Some("bad \u{fffd} byte"));
}

#[test]
fn no_reason_for_other_content_types_charsets_or_an_empty_body() {
    assert_eq!(reason_of(Some("application/json"), b"{\"e\":1}"), None);
    assert_eq!(reason_of(Some("text/html"), b"<p>busy</p>"), None);
    assert_eq!(
        reason_of(Some("text/plain; charset=latin-1"), b"busy"),
        None
    );
    assert_eq!(reason_of(None, b"busy"), None);
    assert_eq!(reason_of(Some("text/plain"), b" \r\n\x00"), None);
}

#[test]
fn an_auth_rejection_body_never_becomes_a_reason() {
    let mut h = ts_host(vec![refusal(
        401,
        &[
            ("Content-Type", "text/plain"),
            ("WWW-Authenticate", "Basic realm=\"x\""),
        ],
        b"secret hint",
    )]);
    request_times(&mut h, 2);
    assert!(refused(&h.events).is_empty());
    assert!(
        !h.events
            .iter()
            .any(|e| format!("{e:?}").contains("secret hint"))
    );
}

#[test]
fn the_notice_never_carries_header_values() {
    let mut h = ts_host(vec![refusal(
        503,
        &[
            ("Retry-After", "30"),
            ("Content-Type", "text/plain"),
            ("Set-Cookie", "sid=hunter2"),
            ("X-Token", "hunter3"),
        ],
        b"busy",
    )]);
    request_times(&mut h, 2);
    let dump = format!("{:?}", refused(&h.events));
    assert!(!dump.contains("hunter"), "{dump}");
}

#[test]
fn hls_reports_server_refused_and_waits_the_retry_after() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.ok(TS_PLAYLIST_URL, ts_playlist());
    h.route(TS_SEG0_URL, vec![busy("30"), Reply::Ok(ts_segment(0))]);
    h.ok(TS_SEG1_URL, ts_segment(1));
    h.run_until(|_| false, STEPS);
    let notices = refused(&h.events);
    assert_eq!(
        notices,
        vec![(503, Some(30 * SEC), Some(REASON.to_string()))]
    );
    let at: Vec<u64> = h.requests_to(TS_SEG0_URL).iter().map(|s| s.at_us).collect();
    assert_eq!((at[1] - at[0]) / SEC, 30);
}

#[test]
fn dash_reports_server_refused_and_waits_the_retry_after() {
    let mut sim = DashSim::static_origin(DASH_MANIFEST, Config::default());
    sim.fault = Some(Box::new(|req: &Logged, _| {
        req.url.ends_with("init-stream0.m4s").then(|| Answer::Full {
            status: 503,
            headers: vec![
                header("Retry-After", "30"),
                header("Content-Type", "text/plain"),
            ],
            body: REASON.as_bytes().to_vec(),
        })
    }));
    sim.run_for(100 * dash_sim::SEC);
    let notices = refused(&sim.events);
    assert!(!notices.is_empty());
    for n in &notices {
        assert_eq!(n, &(503, Some(30 * SEC), Some(REASON.to_string())));
    }
    let at: Vec<u64> = sim
        .requests_to("init-stream0")
        .iter()
        .map(|l| l.send_elapsed_us)
        .collect();
    assert!(at.len() >= 2);
    assert_eq!((at[1] - at[0]) / SEC, 30);
}

#[test]
fn ts_http_keeps_the_refused_connection_until_the_retry_when_the_host_never_closes_it() {
    let mut s = Session::open(TS_URL, Config::default(), 0).unwrap();
    let conn = match s.poll(0) {
        Some(Action::HttpRequest { conn, .. }) => conn,
        other => panic!("expected the first request, got {other:?}"),
    };
    let headers = vec![
        header("Retry-After", "30"),
        header("Content-Type", "text/plain"),
    ];
    s.on_response(conn, 503, &headers, 0);
    let mut actions = Vec::new();
    while let Some(a) = s.poll(0) {
        actions.push(a);
    }
    assert!(
        !actions.iter().any(|a| matches!(a, Action::Close { .. })),
        "the body is still wanted: {actions:?}"
    );
    s.on_data(conn, REASON.as_bytes(), SEC / 100);
    assert!(
        refused_events(&mut s).is_empty(),
        "not announced before the connection ends"
    );
    let mut closed = false;
    let mut requested = false;
    while let Some(a) = s.poll(30 * SEC) {
        closed |= matches!(a, Action::Close { conn: c } if c == conn);
        requested |= matches!(a, Action::HttpRequest { .. });
    }
    assert!(closed && requested);
    assert_eq!(
        refused_events(&mut s),
        vec![(503, Some(30 * SEC), Some(REASON.to_string()))]
    );
}

fn refused_events(s: &mut Session) -> Vec<(u16, Option<u64>, Option<String>)> {
    let mut events = Vec::new();
    while let Some(e) = s.next_event() {
        events.push(e);
    }
    refused(&events)
}
