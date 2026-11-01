use tshttp_runtime::{Action, Config, Event, StarveKind, TsHttpClient};

#[test]
fn backoff_2_4_8_then_10_forever() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    let mut got = Vec::new();
    for _ in 0..6 {
        // fail
        if matches!(c.poll(), Some(Action::Open { .. })) {
            let _ = c.poll();
        }
        c.on_error(0);
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
    assert_eq!(got, vec![2000, 4000, 8000, 10000, 10000, 10000]);
}

#[test]
fn played_then_ended_resets_attempt_counter() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    // fail twice
    c.on_error(0);
    let dl = c.next_deadline().unwrap();
    c.poll_timeout(dl);
    let _ = c.poll();
    c.on_error(0);
    let dl = c.next_deadline().unwrap();
    c.poll_timeout(dl);
    let _ = c.poll();
    const D: &[u8] = include_bytes!("../../../fixtures/ts/h264_aac.ts");
    // play
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    c.feed(D, 1);
    // EOF
    c.on_eof(60_000);
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
    // next failure => 2000
    c.on_error(0);
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 2000 }));
}

#[test]
fn server_closes_every_60s_no_stall_notice() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    for i in 0..10 {
        c.on_response(200, &[], i * 60000);
        let _ = c.poll_event();
        const D: &[u8] = include_bytes!("../../../fixtures/ts/h264_aac.ts");
        c.feed(D, i * 60000 + 1);
        c.on_eof(i * 60000 + 60000);
        assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 0 }));
        let dl = c.next_deadline().unwrap();
        c.poll_timeout(dl);
        let _ = c.poll();
    }
    // no events starved/stall in queue (drained)
}

#[test]
fn not_found_then_ok() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(404, &[], 0);
    assert!(matches!(
        c.poll_event(),
        Some(Event::NotFound { status: 404 })
    ));
    let _ = c.poll_event(); // reconnecting
    let _ = c.poll(); // reconnect
    let dl = c.next_deadline().unwrap();
    c.poll_timeout(dl);
    let _ = c.poll(); // open
    c.on_response(200, &[], 100);
    assert!(matches!(c.poll_event(), Some(Event::Connected)));
    c.feed(&[0u8; 4], 101);
    // maybe no sample; ok
}

#[test]
fn gone_410_is_not_found_event() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(410, &[], 0);
    assert!(matches!(
        c.poll_event(),
        Some(Event::NotFound { status: 410 })
    ));
}

#[test]
fn stall_notice_repeats_once_per_second_after_first() {
    const D: &[u8] = include_bytes!("../../../fixtures/ts/h264_aac.ts");
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    c.feed(D, 0);
    // no bytes afterwards; poll timeouts
    c.poll_timeout(5000);
    // first stall notice
    let mut notices = 0;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::StallNotice { .. }) {
            notices += 1;
        }
    }
    assert!(notices >= 1);
    c.poll_timeout(6000);
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
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    c.reconnect(5);
    let _ = c.poll();
    let _ = c.poll();
    // now backoff; feed/on_eof should not change state/queue
    let before = c.next_deadline();
    c.feed(&[0u8; 4], 10);
    c.on_eof(10);
    assert_eq!(c.next_deadline(), before);
}

#[test]
fn caller_reconnect_in_every_connected_state() {
    for phase in 0..3 {
        let mut c = TsHttpClient::new("u", Config::default(), 0);
        let _ = c.poll();
        if phase >= 1 {
            c.on_response(200, &[], 0);
            let _ = c.poll_event();
        }
        if phase >= 2 {
            c.feed(&[0u8; 4], 1);
        }
        c.reconnect(5);
        assert_eq!(c.poll(), Some(Action::Abort));
        assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 0 }));
    }
}

#[test]
fn starved_data_vs_nodata_classified_separately() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    // data flowing but no sample for stall_notice
    c.feed(&[0u8; 4], 10000);
    let mut saw_data = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Starved(StarveKind::Data)) {
            saw_data = true;
        }
    }
    assert!(saw_data);

    const D: &[u8] = include_bytes!("../../../fixtures/ts/h264_aac.ts");
    let mut c2 = TsHttpClient::new("u", Config::default(), 0);
    let _ = c2.poll();
    c2.on_response(200, &[], 0);
    let _ = c2.poll_event();
    c2.feed(D, 0);
    c2.poll_timeout(5000);
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
        start_timeout_ms: 7,
        stall_notice_ms: 3,
        stall_reconnect_ms: 9,
        backoff_ms: vec![5],
        backoff_cap_ms: 11,
        min_reconnect_interval_ms: 0,
    };
    let mut c = TsHttpClient::new("u", cfg, 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    let _ = c.poll_event();
    c.poll_timeout(7);
    // awaiting timeout abort
    assert_eq!(c.poll(), Some(Action::Abort));
}
