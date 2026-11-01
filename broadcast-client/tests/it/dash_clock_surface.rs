//! The clock and configuration surface DASH needs (`docs/dash-runtime/DESIGN.md` 3.5, 6.3, 6.4):
//! `Session::set_wall_clock`, the HTTP-date form of `Retry-After`, `Config::dash`,
//! `Event::ClockSynced` and the new `ErrorKind`s. Emission of `ClockSynced` and of the DASH error
//! kinds lands with the DASH source; here only the types and the clock plumbing are exercised.

use crate::support;

use broadcast_client::{
    Action, ClockSource, Config, DashClockPolicy, DashConfig, Error, ErrorKind, Event, Session,
};
use support::hls_host::{
    Host, Reply, SEC, TS_PLAYLIST_URL, TS_SEG0_URL, TS_SEG1_URL, hls_config, is_ended, ts_playlist,
    ts_segment,
};

const URL: &str = "http://h/live.m3u8";
const STEPS: usize = 40_000;
const FIRST_BACKOFF_SECS: u64 = 2;
const MAX_RETRY_AFTER_SECS: u64 = 60;
const BASE_UNIX_US: u64 = 1_793_003_944 * SEC;
const BASE_DATE: &str = "Mon, 26 Oct 2026 08:39:04 GMT";
const PLUS_30S: &str = "Mon, 26 Oct 2026 08:39:34 GMT";
const MINUS_1S: &str = "Mon, 26 Oct 2026 08:39:03 GMT";
const PLUS_1H: &str = "Mon, 26 Oct 2026 09:39:04 GMT";

fn header(name: &str, value: &str) -> (String, String) {
    (name.to_string(), value.to_string())
}

fn retry_after(status: u16, value: &str) -> Reply {
    Reply::StatusHeaders(status, vec![header("Retry-After", value)])
}

fn retry_after_with_date(value: &str, date: &str) -> Reply {
    Reply::StatusHeaders(
        503,
        vec![header("Date", date), header("Retry-After", value)],
    )
}

fn hls_host_with_first_segment_reply(reply: Reply) -> Host {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.ok(TS_PLAYLIST_URL, ts_playlist());
    h.route(TS_SEG0_URL, vec![reply, Reply::Ok(ts_segment(0))]);
    h.ok(TS_SEG1_URL, ts_segment(1));
    h
}

fn first_gap_secs(h: &mut Host) -> u64 {
    h.run_until(
        |e| {
            e.iter()
                .any(|x| is_ended(x) || matches!(x, Event::Error { fatal: true, .. }))
        },
        STEPS,
    );
    assert!(h.events.iter().any(is_ended), "the stream ended cleanly");
    let at: Vec<u64> = h.requests_to(TS_SEG0_URL).iter().map(|s| s.at_us).collect();
    assert!(at.len() >= 2, "the segment was retried: {at:?}");
    (at[1] - at[0]) / SEC
}

fn gap_with_anchor(anchors: &[(u64, u64)], reply: Reply) -> u64 {
    let mut h = hls_host_with_first_segment_reply(reply);
    for (mono, unix) in anchors {
        h.session.set_wall_clock(*mono, *unix);
    }
    first_gap_secs(&mut h)
}

type Anchors = &'static [(u64, u64)];

#[test]
fn an_http_date_retry_after_resolves_against_the_anchor_or_the_response_date() {
    let anchor: Anchors = &[(0, BASE_UNIX_US)];
    let cases: [(Anchors, Reply, u64, &str); 7] = [
        (anchor, retry_after(503, PLUS_30S), 30, "anchor, 503"),
        (anchor, retry_after(429, PLUS_30S), 30, "anchor, 429"),
        (
            anchor,
            retry_after(503, MINUS_1S),
            FIRST_BACKOFF_SECS,
            "in the past",
        ),
        (
            anchor,
            retry_after(503, BASE_DATE),
            FIRST_BACKOFF_SECS,
            "now is not in the future",
        ),
        (
            anchor,
            retry_after(503, PLUS_1H),
            MAX_RETRY_AFTER_SECS,
            "far future is clamped",
        ),
        (
            &[],
            retry_after(503, PLUS_30S),
            FIRST_BACKOFF_SECS,
            "no anchor and no Date header is ignored",
        ),
        (
            &[],
            retry_after_with_date(PLUS_30S, BASE_DATE),
            30,
            "no anchor counts from the response Date",
        ),
    ];
    for (anchors, reply, want, why) in cases {
        assert_eq!(gap_with_anchor(anchors, reply), want, "{why}");
    }
}

#[test]
fn the_anchor_wins_over_a_skewed_response_date() {
    let skewed_server = "Sun, 01 Jan 2023 00:00:00 GMT";
    let by_date = gap_with_anchor(&[], retry_after_with_date(PLUS_30S, skewed_server));
    assert_eq!(
        by_date, MAX_RETRY_AFTER_SECS,
        "no anchor: the Date is the reference"
    );
    let by_anchor = gap_with_anchor(
        &[(0, BASE_UNIX_US)],
        retry_after_with_date(PLUS_30S, skewed_server),
    );
    assert_eq!(by_anchor, 30);
}

#[test]
fn the_latest_anchor_replaces_the_previous_one() {
    let two_calls = gap_with_anchor(
        &[(0, BASE_UNIX_US + 100 * SEC), (0, BASE_UNIX_US)],
        retry_after(503, PLUS_30S),
    );
    assert_eq!(two_calls, 30, "the second call won");
    let reversed = gap_with_anchor(
        &[(0, BASE_UNIX_US), (0, BASE_UNIX_US + 100 * SEC)],
        retry_after(503, PLUS_30S),
    );
    assert_eq!(reversed, FIRST_BACKOFF_SECS, "the second call won again");
    let same_twice = gap_with_anchor(
        &[(0, BASE_UNIX_US), (0, BASE_UNIX_US)],
        retry_after(503, PLUS_30S),
    );
    assert_eq!(same_twice, 30);
}

#[test]
fn an_anchor_taken_after_the_response_instant_still_maps_by_offset() {
    let backwards = gap_with_anchor(&[(1_000 * SEC, BASE_UNIX_US)], retry_after(503, PLUS_30S));
    assert_eq!(
        backwards, MAX_RETRY_AFTER_SECS,
        "the response at 0 is 1000 s before the anchor, so the date is 1030 s away"
    );
    let earlier_than_before = gap_with_anchor(
        &[(500 * SEC, BASE_UNIX_US), (0, BASE_UNIX_US)],
        retry_after(503, PLUS_30S),
    );
    assert_eq!(earlier_than_before, 30);
}

#[test]
fn extreme_anchors_are_accepted_and_do_not_wrap() {
    let cases = [
        (
            (u64::MAX, u64::MAX),
            MAX_RETRY_AFTER_SECS,
            "UTC at 0 is 1970",
        ),
        (
            (0, u64::MAX),
            FIRST_BACKOFF_SECS,
            "UTC is the far future: the date passed",
        ),
        ((u64::MAX, 0), MAX_RETRY_AFTER_SECS, "UTC far before 1970"),
        ((0, 0), MAX_RETRY_AFTER_SECS, "UTC is 1970"),
        (
            (u64::MAX, BASE_UNIX_US),
            MAX_RETRY_AFTER_SECS,
            "UTC far before 1970",
        ),
    ];
    for ((mono, unix), want, why) in cases {
        let gap = gap_with_anchor(&[(mono, unix)], retry_after(503, PLUS_30S));
        assert_eq!(gap, want, "anchor ({mono}, {unix}): {why}");
    }
}

#[test]
fn delta_seconds_and_garbage_behave_as_before_with_an_anchor_set() {
    let cases = [
        ("30", 30),
        ("7", 7),
        ("61", MAX_RETRY_AFTER_SECS),
        ("soon", FIRST_BACKOFF_SECS),
        ("-5", FIRST_BACKOFF_SECS),
        ("", FIRST_BACKOFF_SECS),
        ("Mon, 99 Oct 2026 08:39:34 GMT", FIRST_BACKOFF_SECS),
    ];
    for (value, want) in cases {
        let gap = gap_with_anchor(&[(0, BASE_UNIX_US)], retry_after(503, value));
        assert_eq!(gap, want, "{value:?}");
    }
}

#[test]
fn set_wall_clock_after_close_is_harmless() {
    let mut s = Session::open(URL, hls_config(), 0).unwrap();
    s.close();
    s.set_wall_clock(1, 2);
    s.set_wall_clock(u64::MAX, u64::MAX);
    assert!(s.next_event().is_none());
}

#[test]
fn set_wall_clock_emits_nothing_by_itself() {
    let mut s = Session::open(URL, hls_config(), 0).unwrap();
    let mut control = Session::open(URL, hls_config(), 0).unwrap();
    s.set_wall_clock(0, BASE_UNIX_US);
    s.set_wall_clock(0, BASE_UNIX_US);
    let drain = |s: &mut Session| {
        let mut actions = Vec::new();
        while let Some(a) = s.poll(0) {
            actions.push(a);
        }
        let mut events = Vec::new();
        while let Some(e) = s.next_event() {
            events.push(e);
        }
        (actions, events)
    };
    assert_eq!(drain(&mut s), drain(&mut control));
}

#[test]
fn a_ts_http_session_ignores_the_anchor_even_with_an_http_date_retry_after() {
    let run = |anchor: bool| {
        let mut s = Session::open("http://h/live.ts", Config::default(), 0).unwrap();
        if anchor {
            s.set_wall_clock(0, BASE_UNIX_US);
        }
        let mut seen: Vec<Action> = Vec::new();
        let mut events = Vec::new();
        let mut now = 0u64;
        for _ in 0..8 {
            while let Some(a) = s.poll(now) {
                if let Action::HttpRequest { conn, .. } = &a {
                    s.on_response(
                        *conn,
                        503,
                        &[header("Retry-After", PLUS_30S), header("Date", BASE_DATE)],
                        now,
                    );
                    s.on_closed(*conn, None, now);
                }
                seen.push(a);
            }
            while let Some(e) = s.next_event() {
                events.push(format!("{e:?}"));
            }
            now += 5 * SEC;
        }
        (seen, events)
    };
    let with = run(true);
    let without = run(false);
    assert!(
        with.0
            .iter()
            .filter(|a| matches!(a, Action::HttpRequest { .. }))
            .count()
            >= 2,
        "the script retried at least once"
    );
    assert_eq!(with, without);
}

#[test]
fn no_source_emits_clock_synced_before_the_dash_source_exists() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.ok(TS_PLAYLIST_URL, ts_playlist());
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(TS_SEG1_URL, ts_segment(1));
    h.session.set_wall_clock(0, BASE_UNIX_US);
    h.run_to_end();
    assert!(h.events.iter().any(is_ended));
    assert!(
        !h.events
            .iter()
            .any(|e| matches!(e, Event::ClockSynced { .. }))
    );
}

fn field_of(config: Config) -> Option<&'static str> {
    match Session::open(URL, config, 0) {
        Err(Error::InvalidConfig { field }) => Some(field),
        Err(other) => panic!("unexpected error {other}"),
        Ok(_) => None,
    }
}

fn with_dash(edit: impl FnOnce(&mut DashConfig)) -> Config {
    let mut config = Config::default();
    edit(&mut config.dash);
    config
}

#[test]
fn the_default_dash_config_is_the_dash_default_and_opens() {
    assert_eq!(Config::default().dash, DashConfig::default());
    assert_eq!(field_of(Config::default()), None);
    assert!(format!("{:?}", Config::default()).contains("dash: DashConfig"));
}

#[test]
fn every_invalid_dash_field_is_rejected_by_its_dotted_name() {
    type Edit = fn(&mut DashConfig);
    let cases: [(&str, Edit); 6] = [
        ("dash.utc_timeout_us", |d| d.utc_timeout_us = 0),
        ("dash.utc_wait_us", |d| d.utc_wait_us = 0),
        ("dash.utc_max_rtt_us", |d| d.utc_max_rtt_us = 0),
        ("dash.utc_resync_us", |d| d.utc_resync_us = 0),
        ("dash.early_retry_us", |d| d.early_retry_us = 0),
        ("dash.min_mpd_refresh_us", |d| d.min_mpd_refresh_us = 0),
    ];
    for (name, edit) in cases {
        assert_eq!(field_of(with_dash(edit)), Some(name), "{name}");
    }
}

#[test]
fn nonsense_combinations_are_rejected() {
    let wait_shorter_than_one_request = with_dash(|d| {
        d.utc_timeout_us = 4 * SEC;
        d.utc_wait_us = 4 * SEC - 1;
    });
    assert_eq!(
        field_of(wait_shorter_than_one_request),
        Some("dash.utc_wait_us")
    );
    let equal = with_dash(|d| {
        d.utc_timeout_us = 4 * SEC;
        d.utc_wait_us = 4 * SEC;
    });
    assert_eq!(field_of(equal), None);
}

#[test]
fn the_largest_values_and_the_zero_meaning_none_open() {
    let config = with_dash(|d| {
        d.utc_timeout_us = u64::MAX;
        d.utc_wait_us = u64::MAX;
        d.utc_max_rtt_us = u64::MAX;
        d.utc_resync_us = u64::MAX;
        d.host_clock_uncertainty_us = u64::MAX;
        d.live_delay_us = Some(u64::MAX);
        d.availability_margin_us = u64::MAX;
        d.early_retry_us = u64::MAX;
        d.early_grace_us = u64::MAX;
        d.min_mpd_refresh_us = u64::MAX;
    });
    assert_eq!(field_of(config), None);
    let zeros = with_dash(|d| {
        d.host_clock_uncertainty_us = 0;
        d.live_delay_us = Some(0);
        d.availability_margin_us = 0;
        d.early_grace_us = 0;
    });
    assert_eq!(field_of(zeros), None);
}

#[test]
fn the_policy_and_switches_never_invalidate() {
    for policy in [DashClockPolicy::Auto, DashClockPolicy::PreferHost] {
        for flag in [false, true] {
            let config = with_dash(|d| {
                d.clock_policy = policy;
                d.use_availability_time_offset = flag;
                d.cross_origin_urls = flag;
            });
            assert_eq!(field_of(config), None);
        }
    }
}

#[test]
fn an_invalid_dash_config_fails_a_session_that_does_not_play_dash() {
    let mut config = with_dash(|d| d.early_retry_us = 0);
    config.kind = broadcast_client::SourceKind::TsHttp;
    assert!(matches!(
        Session::open("http://h/live.ts", config, 0),
        Err(Error::InvalidConfig {
            field: "dash.early_retry_us"
        })
    ));
}

#[test]
fn an_earlier_invalid_field_is_reported_before_a_dash_one() {
    let mut config = with_dash(|d| d.utc_timeout_us = 0);
    config.max_queued_events = 0;
    assert_eq!(field_of(config), Some("max_queued_events"));
}

#[test]
fn the_new_error_kinds_have_stable_names_and_display() {
    let cases = [
        (ErrorKind::MpdParse { reason: "xml" }, "mpd_parse"),
        (
            ErrorKind::MpdInvalid {
                reason: "no_period",
            },
            "mpd_invalid",
        ),
        (
            ErrorKind::UnsupportedAddressing { reason: "data_url" },
            "unsupported_addressing",
        ),
        (
            ErrorKind::UnsupportedFeature { feature: "xlink" },
            "unsupported_feature",
        ),
        (ErrorKind::LiveEdgeLost, "live_edge_lost"),
        (ErrorKind::ClockUnavailable, "clock_unavailable"),
    ];
    for (kind, name) in cases {
        assert_eq!(kind.name(), name);
        assert_eq!(kind.to_string(), name);
    }
}

#[test]
fn the_new_error_kinds_compare_by_payload() {
    assert_eq!(
        ErrorKind::MpdParse { reason: "xml" },
        ErrorKind::MpdParse { reason: "xml" }
    );
    assert_ne!(
        ErrorKind::MpdParse { reason: "xml" },
        ErrorKind::MpdInvalid { reason: "xml" }
    );
    assert_ne!(
        ErrorKind::UnsupportedFeature { feature: "a" },
        ErrorKind::UnsupportedFeature { feature: "b" }
    );
    assert_ne!(ErrorKind::LiveEdgeLost, ErrorKind::ClockUnavailable);
}

#[test]
fn clock_synced_compares_by_source_and_uncertainty() {
    let a = Event::ClockSynced {
        source: ClockSource::UtcTiming,
        uncertainty_us: 1_500,
    };
    assert_eq!(
        a,
        Event::ClockSynced {
            source: ClockSource::UtcTiming,
            uncertainty_us: 1_500
        }
    );
    assert_ne!(
        a,
        Event::ClockSynced {
            source: ClockSource::Host,
            uncertainty_us: 1_500
        }
    );
    assert_ne!(
        a,
        Event::ClockSynced {
            source: ClockSource::UtcTiming,
            uncertainty_us: 1_501
        }
    );
    assert_ne!(a, Event::Discontinuity);
    assert!(format!("{a:?}").contains("UtcTiming"));
}

#[test]
fn the_clock_source_labels_reach_the_host() {
    let labels: Vec<String> = [
        ClockSource::MpdDate,
        ClockSource::Host,
        ClockSource::UtcTiming,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(labels, ["mpd_date", "host", "utc_timing"]);
    assert_eq!(DashClockPolicy::PreferHost.to_string(), "prefer_host");
}

#[test]
fn the_not_hls_error_names_both_adaptive_sources() {
    assert!(Error::NotHls.to_string().contains("HLS or DASH"));
    let mut s = Session::open("http://h/live.ts", Config::default(), 0).unwrap();
    assert!(matches!(s.select_variant(0), Err(Error::NotHls)));
    assert!(matches!(s.select_audio_rendition(None), Err(Error::NotHls)));
}
