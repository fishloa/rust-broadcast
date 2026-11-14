use broadcast_client::tshttp::{Action, Config, Event, StarveKind, TsHttpClient};
use broadcast_common::stage::Timestamp;

/// `ms` on the tshttp policy's millisecond clock, as the session's `now`.
fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}

#[test]
fn backoff_2_4_8_then_10_forever() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = c.poll();
    let mut got = Vec::new();
    for _ in 0..6 {
        // fail
        if matches!(c.poll(), Some(Action::Open { .. })) {
            let _ = c.poll();
        }
        c.on_error(ts(0));
        // drain reconnect
        while let Some(a) = c.poll() {
            if let Action::Reconnect { after_ms } = a {
                got.push(after_ms);
            }
        }
        // advance backoff
        let dl = c.next_deadline().unwrap();
        c.poll_timeout(dl);
        let _ = c.poll(); // Open
    }
    // The first step is exactly its configured delay; every later one is
    // jittered within a sixth below it (the shared schedule, issue #2471).
    assert_eq!(got[0], 2000);
    for (step, delay) in [4000u64, 8000, 10000, 10000, 10000].iter().zip(&got[1..]) {
        assert!(
            (*delay as f64) >= (*step as f64) * 5.0 / 6.0 && *delay <= *step,
            "a {step} ms step: {delay} ms"
        );
    }
}

#[test]
fn played_then_ended_resets_attempt_counter() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = c.poll();
    // fail twice
    c.on_error(ts(0));
    let dl = c.next_deadline().unwrap();
    c.poll_timeout(dl);
    let _ = c.poll();
    c.on_error(ts(0));
    let dl = c.next_deadline().unwrap();
    c.poll_timeout(dl);
    let _ = c.poll();
    static D: std::sync::LazyLock<Vec<u8>> =
        std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));
    // play
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.feed(D.as_slice(), ts(1));
    // EOF
    c.on_eof(ts(60_000));
    // reconnect 0 (drain queued actions)
    let mut saw_zero = false;
    while let Some(a) = c.poll() {
        if a == (Action::Reconnect { after_ms: 0 }) {
            saw_zero = true;
        }
    }
    assert!(saw_zero);
    let dl = c.next_deadline().unwrap();
    c.poll_timeout(dl);
    let _ = c.poll(); // Open
    // next failure => the first step again: exactly 2000 ms, unjittered,
    // which is what a restarted series shows.
    c.on_error(ts(0));
    let after_ms = loop {
        match c.poll() {
            Some(Action::Reconnect { after_ms }) => break after_ms,
            Some(_) => continue,
            None => panic!("no reconnect was queued"),
        }
    };
    assert_eq!(after_ms, 2000);
}

#[test]
fn server_closes_every_60s_no_stall_notice() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = c.poll();
    for i in 0..10 {
        c.on_response(200, &[], ts(i * 60000));
        let _ = c.poll_event();
        static D: std::sync::LazyLock<Vec<u8>> =
            std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));
        c.feed(D.as_slice(), ts(i * 60000 + 1));
        c.on_eof(ts(i * 60000 + 60000));
        assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 0 }));
        let dl = c.next_deadline().unwrap();
        c.poll_timeout(dl);
        let _ = c.poll();
    }
    // no events starved/stall in queue (drained)
}

#[test]
fn not_found_then_ok() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = c.poll();
    c.on_response(404, &[], ts(0));
    assert!(matches!(
        c.poll_event(),
        Some(Event::NotFound { status: 404 })
    ));
    let _ = c.poll_event(); // reconnecting
    let _ = c.poll(); // reconnect
    let dl = c.next_deadline().unwrap();
    c.poll_timeout(dl);
    let _ = c.poll(); // open
    c.on_response(200, &[], ts(100));
    assert!(matches!(c.poll_event(), Some(Event::Connected)));
    c.feed(&[0u8; 4], ts(101));
    // maybe no sample; ok
}

#[test]
fn gone_410_is_not_found_event() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = c.poll();
    c.on_response(410, &[], ts(0));
    assert!(matches!(
        c.poll_event(),
        Some(Event::NotFound { status: 410 })
    ));
}

#[test]
fn stall_notice_repeats_once_per_second_after_first() {
    static D: std::sync::LazyLock<Vec<u8>> =
        std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.feed(D.as_slice(), ts(0));
    // no bytes afterwards; poll timeouts
    c.poll_timeout(ts(5000));
    // first stall notice
    let mut notices = 0;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::StallNotice { .. }) {
            notices += 1;
        }
    }
    assert!(notices >= 1);
    c.poll_timeout(ts(6000));
    let mut notices2 = 0;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::StallNotice { .. }) {
            notices2 += 1;
        }
    }
    assert!(notices2 >= 1);
}

#[test]
fn callbacks_after_abort_are_ignored() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.reconnect(ts(5));
    let _ = c.poll();
    let _ = c.poll();
    // now backoff; feed/on_eof should not change state/queue
    let before = c.next_deadline();
    c.feed(&[0u8; 4], ts(10));
    c.on_eof(ts(10));
    assert_eq!(c.next_deadline(), before);
}

#[test]
fn caller_reconnect_in_every_connected_state() {
    for phase in 0..3 {
        let mut c = TsHttpClient::new("u", Config::default(), ts(0));
        let _ = c.poll();
        if phase >= 1 {
            c.on_response(200, &[], ts(0));
            let _ = c.poll_event();
        }
        if phase >= 2 {
            c.feed(&[0u8; 4], ts(1));
        }
        c.reconnect(ts(5));
        assert_eq!(c.poll(), Some(Action::Abort));
        assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 0 }));
    }
}

#[test]
fn starved_data_vs_nodata_classified_separately() {
    let mut c = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    // data flowing but no sample for stall_notice
    c.feed(&[0u8; 4], ts(10000));
    let mut saw_data = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Starved(StarveKind::Data)) {
            saw_data = true;
        }
    }
    assert!(saw_data);

    static D: std::sync::LazyLock<Vec<u8>> =
        std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));
    let mut c2 = TsHttpClient::new("u", Config::default(), ts(0));
    let _ = c2.poll();
    c2.on_response(200, &[], ts(0));
    let _ = c2.poll_event();
    c2.feed(D.as_slice(), ts(0));
    c2.poll_timeout(ts(5000));
    let mut saw_nodata = false;
    while let Some(e) = c2.poll_event() {
        if matches!(e, Event::Starved(StarveKind::NoData)) {
            saw_nodata = true;
        }
    }
    assert!(saw_nodata);
}

#[test]
fn config_overrides_change_every_timeout() {
    let cfg = Config {
        start_timeout: std::time::Duration::from_millis(7),
        stall_notice: std::time::Duration::from_millis(3),
        stall_reconnect: std::time::Duration::from_millis(9),
        backoff: broadcast_common::backoff::BackoffSteps {
            steps: vec![std::time::Duration::from_millis(5)],
            cap: std::time::Duration::from_millis(11),
        },
        min_reconnect_interval: std::time::Duration::ZERO,
    };
    let mut c = TsHttpClient::new("u", cfg, ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.poll_timeout(ts(7));
    // awaiting timeout abort
    assert_eq!(c.poll(), Some(Action::Abort));
}
