use broadcast_client::tshttp::{Action, Config, Event, TsHttpClient};
use broadcast_common::backoff::BackoffSteps;
use broadcast_common::stage::Timestamp;
use std::time::Duration;

/// The harness's millisecond clock as the session's `now`: this module's instants (`now`,
/// `14_999`, `next_deadline()`) are milliseconds, the policy's own unit.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_millis(us)
}

static D: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));
const TEN_MINUTES_MS: u64 = 600_000;
const ITERATION_BOUND: u32 = 5_000;

fn actions(c: &mut TsHttpClient) -> Vec<Action> {
    let mut v = Vec::new();
    while let Some(a) = c.poll() {
        v.push(a);
    }
    v
}

fn events(c: &mut TsHttpClient) -> Vec<Event> {
    let mut v = Vec::new();
    while let Some(e) = c.poll_event() {
        v.push(e);
    }
    v
}

fn stall_notices(evs: &[Event]) -> Vec<u64> {
    evs.iter()
        .filter_map(|e| match e {
            Event::StallNotice { secs } => Some(*secs),
            _ => None,
        })
        .collect()
}

fn playing(config: Config) -> TsHttpClient {
    let mut c = TsHttpClient::new("u", config, ts(0));
    let _ = actions(&mut c);
    c.on_response(200, &[], ts(0));
    c.feed(D.as_slice(), ts(0));
    let _ = events(&mut c);
    c
}

fn run_host_loop(config: Config, silent_server: bool) -> u32 {
    let mut c = TsHttpClient::new("u", config, ts(0));
    let mut now = 0u64;
    let mut iterations = 0u32;
    while now < TEN_MINUTES_MS {
        iterations += 1;
        assert!(
            iterations <= ITERATION_BOUND,
            "host loop spun {iterations} times by simulated t={now} ms"
        );
        for a in actions(&mut c) {
            if matches!(a, Action::Open { .. }) {
                c.on_response(200, &[], ts(now));
                if silent_server {
                    c.feed(D.as_slice(), ts(now));
                }
            }
        }
        let _ = events(&mut c);
        match c.next_deadline() {
            Some(d) => now = now.max(d.as_millis()),
            None => break,
        }
        c.poll_timeout(ts(now));
    }
    iterations
}

#[test]
fn next_deadline_is_in_the_future_after_the_first_stall_notice() {
    let mut c = playing(Config::default());
    c.poll_timeout(ts(5_000));
    assert_eq!(stall_notices(&events(&mut c)), vec![5]);
    let d = c.next_deadline().expect("playing has a deadline");
    assert!(d > ts(5_000), "deadline {d:?} is not after now=5000");
    assert_eq!(d, ts(6_000));
}

#[test]
fn host_loop_over_ten_minutes_of_silence_is_bounded() {
    let n = run_host_loop(Config::default(), true);
    assert!(n < ITERATION_BOUND, "{n}");
}

#[test]
fn host_loop_with_reconnect_below_notice_is_bounded() {
    let config = Config::default()
        .with_stall_notice(Duration::from_millis(5_000))
        .with_stall_reconnect(Duration::from_millis(3_000));
    let n = run_host_loop(config, true);
    assert!(n < ITERATION_BOUND, "{n}");
}

#[test]
fn opening_has_a_deadline_and_it_fires_the_failure_path() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = actions(&mut c);
    assert_eq!(c.next_deadline(), Some(Timestamp::from_millis(15_000)));
    c.poll_timeout(ts(14_999));
    assert!(actions(&mut c).is_empty());
    c.poll_timeout(ts(15_000));
    assert_eq!(
        actions(&mut c),
        vec![Action::Abort, Action::Reconnect { after_ms: 2_000 }]
    );
    assert!(matches!(
        events(&mut c).as_slice(),
        [Event::Reconnecting { attempt: 1 }]
    ));
    assert_eq!(c.next_deadline(), Some(Timestamp::from_millis(17_000)));
    c.poll_timeout(ts(17_000));
    assert_eq!(actions(&mut c), vec![Action::Open { url: "u".into() }]);
    assert_eq!(c.next_deadline(), Some(Timestamp::from_millis(32_000)));
}

#[test]
fn opening_response_in_time_cancels_the_opening_deadline() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = actions(&mut c);
    c.on_response(200, &[], ts(10_000));
    c.poll_timeout(ts(15_000));
    assert!(actions(&mut c).contains(&Action::Abort));
}

#[test]
fn short_played_connection_cannot_spin() {
    let mut c = playing(Config::default());
    c.on_eof(ts(0));
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 1_000 }]);
    assert_eq!(c.next_deadline(), Some(Timestamp::from_millis(1_000)));
    c.poll_timeout(ts(999));
    assert!(actions(&mut c).is_empty());
    c.poll_timeout(ts(1_000));
    assert_eq!(actions(&mut c), vec![Action::Open { url: "u".into() }]);
}

#[test]
fn short_played_connection_floor_is_open_to_open() {
    let mut c = playing(Config::default());
    c.on_eof(ts(400));
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 600 }]);
}

#[test]
fn short_played_connection_keeps_attempt_reset() {
    let mut c = playing(Config::default());
    c.on_eof(ts(0));
    let _ = actions(&mut c);
    c.poll_timeout(ts(1_000));
    let _ = actions(&mut c);
    c.on_response(500, &[], ts(1_000));
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 2_000 }]);
}

#[test]
fn long_lived_stream_reconnects_with_zero_delay() {
    let mut c = playing(Config::default());
    c.on_eof(ts(60_000));
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 0 }]);
    assert_eq!(c.next_deadline(), Some(Timestamp::from_millis(60_000)));
}

#[test]
fn connection_just_at_the_floor_reconnects_with_zero_delay() {
    let mut c = playing(Config::default());
    c.on_eof(ts(1_000));
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 0 }]);
}

#[test]
fn one_sample_then_close_server_spaces_opens_by_the_floor() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let mut now = 0u64;
    let mut opens = 0u32;
    let mut iterations = 0u32;
    while now < 10_000 {
        iterations += 1;
        assert!(
            iterations <= ITERATION_BOUND,
            "host loop spun {iterations} times by simulated t={now} ms"
        );
        for a in actions(&mut c) {
            if matches!(a, Action::Open { .. }) {
                opens += 1;
                c.on_response(200, &[], ts(now));
                c.feed(D.as_slice(), ts(now));
                c.on_eof(ts(now));
            }
        }
        let _ = events(&mut c);
        match c.next_deadline() {
            Some(d) => now = now.max(d.as_millis()),
            None => break,
        }
        c.poll_timeout(ts(now));
    }
    assert!(opens <= 11, "{opens} opens in 10 s");
    assert!(opens >= 9, "{opens} opens in 10 s");
}

#[test]
fn stall_catch_up_after_one_hour_jump_emits_one_notice() {
    let config = Config::default().with_stall_reconnect(Duration::from_secs(10_000));
    let mut c = playing(config);
    c.poll_timeout(ts(5_000));
    assert_eq!(stall_notices(&events(&mut c)), vec![5]);
    c.poll_timeout(ts(3_605_000));
    assert_eq!(stall_notices(&events(&mut c)), vec![3_605]);
    let d = c.next_deadline().expect("deadline");
    assert!(d > ts(3_605_000), "{d:?}");
    c.poll_timeout(ts(3_605_999));
    assert!(stall_notices(&events(&mut c)).is_empty());
    c.poll_timeout(ts(3_606_000));
    assert_eq!(stall_notices(&events(&mut c)), vec![3_606]);
}

#[test]
fn stall_notice_arithmetic_does_not_overflow_near_u64_max() {
    let config = Config::default().with_stall_reconnect(Duration::from_nanos(u64::MAX));
    let mut c = playing(config);
    c.poll_timeout(ts(u64::MAX - 1));
    assert_eq!(stall_notices(&events(&mut c)).len(), 1);
    c.poll_timeout(ts(u64::MAX - 1));
    assert!(stall_notices(&events(&mut c)).is_empty());
    assert!(c.next_deadline().is_some());
}

fn hdr(k: &str, v: &str) -> Vec<(String, String)> {
    vec![(k.to_string(), v.to_string())]
}

fn finite_after(status: u16, headers: &[(String, String)]) -> Option<u64> {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = actions(&mut c);
    c.on_response(status, headers, ts(0));
    events(&mut c).into_iter().find_map(|e| match e {
        Event::Finite { expected_bytes } => Some(expected_bytes),
        _ => None,
    })
}

#[test]
fn range_end_must_be_below_total() {
    assert_eq!(
        finite_after(206, &hdr("Content-Range", "bytes 0-49/50")),
        Some(50)
    );
    assert_eq!(
        finite_after(206, &hdr("Content-Range", "bytes 0-99/50")),
        None
    );
    assert_eq!(
        finite_after(206, &hdr("Content-Range", "bytes 10-50/50")),
        None
    );
}

/// RFC 9110 §14.4: `*` is a valid complete-length, so the 206 body is still the
/// finite `last - first + 1` bytes; stray whitespace inside the value is not
/// in the grammar.
#[test]
fn an_unknown_complete_length_is_valid_and_stray_whitespace_is_not() {
    assert_eq!(
        finite_after(206, &hdr("Content-Range", "bytes 0-9/*")),
        Some(10)
    );
    assert_eq!(
        finite_after(206, &hdr("Content-Range", "bytes  0-9 / 10")),
        None
    );
}

#[test]
fn content_encoding_other_than_identity_is_not_finite() {
    let cl = |enc: &str| {
        vec![
            ("Content-Length".to_string(), "100".to_string()),
            ("Content-Encoding".to_string(), enc.to_string()),
        ]
    };
    assert_eq!(finite_after(200, &cl("gzip")), None);
    assert_eq!(finite_after(200, &cl("GZIP")), None);
    assert_eq!(finite_after(200, &cl("identity, gzip")), None);
    assert_eq!(finite_after(200, &cl("identity")), Some(100));
    assert_eq!(finite_after(200, &cl("Identity")), Some(100));
    let ranged = vec![
        ("Content-Range".to_string(), "bytes 0-9/10".to_string()),
        ("Content-Encoding".to_string(), "br".to_string()),
    ];
    assert_eq!(finite_after(206, &ranged), None);
}

#[test]
fn bytes_after_a_stall_notice_restart_the_notice_schedule() {
    let mut c = playing(Config::default());
    c.poll_timeout(ts(5_000));
    let _ = events(&mut c);
    c.feed(&[0u8; 188], ts(6_000));
    let _ = events(&mut c);
    let d = c.next_deadline().expect("deadline");
    assert!(d > ts(6_000), "deadline {d:?} is not after now=6000");
    assert_eq!(d, ts(11_000));
}

#[test]
fn validate_rejects_each_inconsistent_field() {
    use broadcast_client::tshttp::config::ConfigError;
    assert_eq!(Config::default().validate(), Ok(()));
    let bad = |f: fn(&mut Config)| {
        let mut c = Config::default();
        f(&mut c);
        c.validate()
    };
    assert_eq!(
        bad(|c| c.start_timeout = Duration::ZERO),
        Err(ConfigError::ZeroStartTimeout)
    );
    assert_eq!(
        bad(|c| c.stall_notice = Duration::ZERO),
        Err(ConfigError::ZeroStallNotice)
    );
    assert_eq!(
        bad(|c| c.stall_reconnect = Duration::ZERO),
        Err(ConfigError::ZeroStallReconnect)
    );
    assert_eq!(
        bad(|c| c.stall_reconnect = c.stall_notice),
        Err(ConfigError::StallReconnectNotAfterNotice)
    );
    assert_eq!(
        bad(|c| c.stall_reconnect = c.stall_notice - Duration::from_millis(1)),
        Err(ConfigError::StallReconnectNotAfterNotice)
    );
    assert_eq!(
        bad(|c| c.backoff.steps.clear()),
        Err(ConfigError::EmptyBackoff)
    );
}

#[test]
fn clamped_config_always_validates_and_leaves_valid_config_alone() {
    assert_eq!(Config::default().clamped(), Config::default());
    let worst = Config::default()
        .with_start_timeout(Duration::ZERO)
        .with_stall_notice(Duration::ZERO)
        .with_stall_reconnect(Duration::ZERO)
        .with_backoff(BackoffSteps {
            steps: Vec::new(),
            cap: Duration::ZERO,
        })
        .with_min_reconnect_interval(Duration::ZERO);
    let c = worst.clamped();
    assert_eq!(c.validate(), Ok(()));
    assert_eq!(
        (c.start_timeout, c.stall_notice, c.stall_reconnect),
        (
            Duration::from_millis(1),
            Duration::from_millis(1),
            Duration::from_millis(2)
        )
    );
    assert_eq!(c.backoff.steps, vec![Duration::from_millis(1)]);
}

#[test]
fn empty_backoff_and_zero_timeouts_do_not_spin_the_client() {
    let config = Config::default()
        .with_start_timeout(Duration::ZERO)
        .with_stall_notice(Duration::ZERO)
        .with_stall_reconnect(Duration::ZERO)
        .with_backoff(BackoffSteps {
            steps: Vec::new(),
            cap: Duration::from_millis(500),
        })
        .with_min_reconnect_interval(Duration::ZERO);
    let mut c = TsHttpClient::new("u", config, ts(0));
    let _ = actions(&mut c);
    c.poll_timeout(ts(1));
    assert_eq!(
        actions(&mut c),
        vec![Action::Abort, Action::Reconnect { after_ms: 500 }]
    );
}
