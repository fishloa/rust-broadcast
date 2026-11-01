use tshttp_runtime::{Action, Config, Event, TsHttpClient};

const D: &[u8] = include_bytes!("../../fixtures/ts/h264_aac.ts");
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
    let mut c = TsHttpClient::new("u", config, 0);
    let _ = actions(&mut c);
    c.on_response(200, &[], 0);
    c.feed(D, 0);
    let _ = events(&mut c);
    c
}

fn run_host_loop(config: Config, silent_server: bool) -> u32 {
    let mut c = TsHttpClient::new("u", config, 0);
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
                c.on_response(200, &[], now);
                if silent_server {
                    c.feed(D, now);
                }
            }
        }
        let _ = events(&mut c);
        match c.next_deadline() {
            Some(d) => now = now.max(d),
            None => break,
        }
        c.poll_timeout(now);
    }
    iterations
}

#[test]
fn next_deadline_is_in_the_future_after_the_first_stall_notice() {
    let mut c = playing(Config::default());
    c.poll_timeout(5_000);
    assert_eq!(stall_notices(&events(&mut c)), vec![5]);
    let d = c.next_deadline().expect("playing has a deadline");
    assert!(d > 5_000, "deadline {d} is not after now=5000");
    assert_eq!(d, 6_000);
}

#[test]
fn host_loop_over_ten_minutes_of_silence_is_bounded() {
    let n = run_host_loop(Config::default(), true);
    assert!(n < ITERATION_BOUND, "{n}");
}

#[test]
fn host_loop_with_reconnect_below_notice_is_bounded() {
    let config = Config {
        stall_notice_ms: 5_000,
        stall_reconnect_ms: 3_000,
        ..Config::default()
    };
    let n = run_host_loop(config, true);
    assert!(n < ITERATION_BOUND, "{n}");
}

#[test]
fn opening_has_a_deadline_and_it_fires_the_failure_path() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = actions(&mut c);
    assert_eq!(c.next_deadline(), Some(15_000));
    c.poll_timeout(14_999);
    assert!(actions(&mut c).is_empty());
    c.poll_timeout(15_000);
    assert_eq!(
        actions(&mut c),
        vec![Action::Abort, Action::Reconnect { after_ms: 2_000 }]
    );
    assert!(matches!(
        events(&mut c).as_slice(),
        [Event::Reconnecting { attempt: 1 }]
    ));
    assert_eq!(c.next_deadline(), Some(17_000));
    c.poll_timeout(17_000);
    assert_eq!(actions(&mut c), vec![Action::Open { url: "u".into() }]);
    assert_eq!(c.next_deadline(), Some(32_000));
}

#[test]
fn opening_response_in_time_cancels_the_opening_deadline() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = actions(&mut c);
    c.on_response(200, &[], 10_000);
    c.poll_timeout(15_000);
    assert!(actions(&mut c).contains(&Action::Abort));
}

#[test]
fn short_played_connection_cannot_spin() {
    let mut c = playing(Config::default());
    c.on_eof(0);
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 1_000 }]);
    assert_eq!(c.next_deadline(), Some(1_000));
    c.poll_timeout(999);
    assert!(actions(&mut c).is_empty());
    c.poll_timeout(1_000);
    assert_eq!(actions(&mut c), vec![Action::Open { url: "u".into() }]);
}

#[test]
fn short_played_connection_floor_is_open_to_open() {
    let mut c = playing(Config::default());
    c.on_eof(400);
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 600 }]);
}

#[test]
fn short_played_connection_keeps_attempt_reset() {
    let mut c = playing(Config::default());
    c.on_eof(0);
    let _ = actions(&mut c);
    c.poll_timeout(1_000);
    let _ = actions(&mut c);
    c.on_response(500, &[], 1_000);
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 2_000 }]);
}

#[test]
fn long_lived_stream_reconnects_with_zero_delay() {
    let mut c = playing(Config::default());
    c.on_eof(60_000);
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 0 }]);
    assert_eq!(c.next_deadline(), Some(60_000));
}

#[test]
fn connection_just_at_the_floor_reconnects_with_zero_delay() {
    let mut c = playing(Config::default());
    c.on_eof(1_000);
    assert_eq!(actions(&mut c), vec![Action::Reconnect { after_ms: 0 }]);
}

#[test]
fn one_sample_then_close_server_spaces_opens_by_the_floor() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
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
                c.on_response(200, &[], now);
                c.feed(D, now);
                c.on_eof(now);
            }
        }
        let _ = events(&mut c);
        match c.next_deadline() {
            Some(d) => now = now.max(d),
            None => break,
        }
        c.poll_timeout(now);
    }
    assert!(opens <= 11, "{opens} opens in 10 s");
    assert!(opens >= 9, "{opens} opens in 10 s");
}

#[test]
fn stall_catch_up_after_one_hour_jump_emits_one_notice() {
    let config = Config {
        stall_reconnect_ms: 10_000_000,
        ..Config::default()
    };
    let mut c = playing(config);
    c.poll_timeout(5_000);
    assert_eq!(stall_notices(&events(&mut c)), vec![5]);
    c.poll_timeout(3_605_000);
    assert_eq!(stall_notices(&events(&mut c)), vec![3_605]);
    let d = c.next_deadline().expect("deadline");
    assert!(d > 3_605_000, "{d}");
    c.poll_timeout(3_605_999);
    assert!(stall_notices(&events(&mut c)).is_empty());
    c.poll_timeout(3_606_000);
    assert_eq!(stall_notices(&events(&mut c)), vec![3_606]);
}

#[test]
fn stall_notice_arithmetic_does_not_overflow_near_u64_max() {
    let config = Config {
        stall_reconnect_ms: u64::MAX,
        ..Config::default()
    };
    let mut c = playing(config);
    c.poll_timeout(u64::MAX - 1);
    assert_eq!(stall_notices(&events(&mut c)).len(), 1);
    c.poll_timeout(u64::MAX - 1);
    assert!(stall_notices(&events(&mut c)).is_empty());
    assert!(c.next_deadline().is_some());
}

fn hdr(k: &str, v: &str) -> Vec<(String, String)> {
    vec![(k.to_string(), v.to_string())]
}

fn finite_after(status: u16, headers: &[(String, String)]) -> Option<u64> {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = actions(&mut c);
    c.on_response(status, headers, 0);
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
    c.poll_timeout(5_000);
    let _ = events(&mut c);
    c.feed(&[0u8; 188], 6_000);
    let _ = events(&mut c);
    let d = c.next_deadline().expect("deadline");
    assert!(d > 6_000, "deadline {d} is not after now=6000");
    assert_eq!(d, 11_000);
}

#[test]
fn validate_rejects_each_inconsistent_field() {
    use tshttp_runtime::config::ConfigError;
    assert_eq!(Config::default().validate(), Ok(()));
    let bad = |f: fn(&mut Config)| {
        let mut c = Config::default();
        f(&mut c);
        c.validate()
    };
    assert_eq!(
        bad(|c| c.start_timeout_ms = 0),
        Err(ConfigError::ZeroStartTimeout)
    );
    assert_eq!(
        bad(|c| c.stall_notice_ms = 0),
        Err(ConfigError::ZeroStallNotice)
    );
    assert_eq!(
        bad(|c| c.stall_reconnect_ms = 0),
        Err(ConfigError::ZeroStallReconnect)
    );
    assert_eq!(
        bad(|c| c.stall_reconnect_ms = c.stall_notice_ms),
        Err(ConfigError::StallReconnectNotAfterNotice)
    );
    assert_eq!(
        bad(|c| c.stall_reconnect_ms = c.stall_notice_ms - 1),
        Err(ConfigError::StallReconnectNotAfterNotice)
    );
    assert_eq!(
        bad(|c| c.backoff_ms.clear()),
        Err(ConfigError::EmptyBackoff)
    );
}

#[test]
fn clamped_config_always_validates_and_leaves_valid_config_alone() {
    assert_eq!(Config::default().clamped(), Config::default());
    let worst = Config {
        start_timeout_ms: 0,
        stall_notice_ms: 0,
        stall_reconnect_ms: 0,
        backoff_ms: Vec::new(),
        backoff_cap_ms: 0,
        min_reconnect_interval_ms: 0,
    };
    let c = worst.clamped();
    assert_eq!(c.validate(), Ok(()));
    assert_eq!(
        (c.start_timeout_ms, c.stall_notice_ms, c.stall_reconnect_ms),
        (1, 1, 2)
    );
    assert_eq!(c.backoff_ms, vec![1]);
}

#[test]
fn empty_backoff_and_zero_timeouts_do_not_spin_the_client() {
    let config = Config {
        start_timeout_ms: 0,
        stall_notice_ms: 0,
        stall_reconnect_ms: 0,
        backoff_ms: Vec::new(),
        backoff_cap_ms: 500,
        min_reconnect_interval_ms: 0,
    };
    let mut c = TsHttpClient::new("u", config, 0);
    let _ = actions(&mut c);
    c.poll_timeout(1);
    assert_eq!(
        actions(&mut c),
        vec![Action::Abort, Action::Reconnect { after_ms: 500 }]
    );
}
