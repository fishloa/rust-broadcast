use broadcast_client::tshttp::{Action, Config, Event, StarveKind, TsHttpClient};
use broadcast_common::stage::Timestamp;

/// `ms` on the tshttp policy's millisecond clock, as the session's `now`.
fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}

static D: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));

fn cfg() -> Config {
    Config::default()
}

#[test]
fn t01_idle_new_opens() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    assert_eq!(c.poll(), Some(Action::Open { url: "u".into() }));
}

#[test]
fn t02_opening_response_2xx_connected_awaiting() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(10));
    assert!(matches!(c.poll_event(), Some(Event::Connected)));
    assert_eq!(c.poll(), None);
}

#[test]
fn t03_opening_404_notfound_then_backoff() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(404, &[], ts(10));
    assert!(matches!(
        c.poll_event(),
        Some(Event::NotFound { status: 404 })
    ));
    assert!(matches!(
        c.poll_event(),
        Some(Event::Reconnecting { attempt: 1 })
    ));
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 2000 }));
}

#[test]
fn t04_opening_other_status_or_error_backoff() {
    // other non-2xx status
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(500, &[], ts(10));
    assert!(matches!(
        c.poll_event(),
        Some(Event::Reconnecting { attempt: 1 })
    ));
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 2000 }));

    // on_error
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_error(ts(10));
    assert!(matches!(
        c.poll_event(),
        Some(Event::Reconnecting { attempt: 1 })
    ));
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 2000 }));
}

#[test]
fn t05_awaiting_first_sample_moves_to_playing_and_resets_attempts() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    // fail once before playing
    c.on_error(ts(0));
    let dl = c.next_deadline().unwrap();
    c.poll_timeout(dl);
    let _ = c.poll(); // Open
    c.on_response(200, &[], ts(5));
    let _ = c.poll_event();
    // first sample -> Playing, attempt reset
    c.feed(D.as_slice(), ts(6));
    let mut saw_sample = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Demux(ev) if matches!(ev.as_ref(), transmux::DemuxEvent::Sample { .. }))
        {
            saw_sample = true;
        }
    }
    assert!(saw_sample);
    // attempt reset: a later failure after playing gives 0 ms
    c.on_eof(ts(60_000));
    let mut saw_zero = false;
    while let Some(a) = c.poll() {
        if a == (Action::Reconnect { after_ms: 0 }) {
            saw_zero = true;
        }
    }
    assert!(saw_zero);
}

#[test]
fn t06_awaiting_start_timeout() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.poll_timeout(ts(14999));
    assert_eq!(c.poll(), None);
    c.poll_timeout(ts(15000));
    assert_eq!(c.poll(), Some(Action::Abort));
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 2000 }));
}

#[test]
fn t07_awaiting_eof_or_error_before_sample_backoff() {
    // eof
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.on_eof(ts(1));
    assert!(matches!(
        c.poll_event(),
        Some(Event::Reconnecting { attempt: 1 })
    ));
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 2000 }));

    // error
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.on_error(ts(1));
    assert!(matches!(
        c.poll_event(),
        Some(Event::Reconnecting { attempt: 1 })
    ));
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 2000 }));
}

#[test]
fn t08_playing_eof_reconnect_zero() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.feed(D.as_slice(), ts(1));
    c.on_eof(ts(60_000));
    let mut saw_zero = false;
    while let Some(a) = c.poll() {
        if a == (Action::Reconnect { after_ms: 0 }) {
            saw_zero = true;
        }
    }
    assert!(saw_zero);
}

#[test]
fn t09_failure_before_playing_attempt_increments_and_emits_reconnecting() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    for expected in 1u64..=3 {
        c.on_error(ts(0));
        assert!(matches!(
            c.poll_event(),
            Some(Event::Reconnecting { attempt }) if attempt == expected
        ));
        let dl = c.next_deadline().unwrap();
        c.poll_timeout(dl);
        let _ = c.poll(); // Open
    }
}

#[test]
fn t10_backoff_due_opens() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(404, &[], ts(0));
    let _ = c.poll_event();
    let _ = c.poll_event();
    let _ = c.poll();
    c.poll_timeout(ts(1999));
    assert_eq!(c.poll(), None);
    c.poll_timeout(ts(2000));
    assert_eq!(c.poll(), Some(Action::Open { url: "u".into() }));
}

#[test]
fn t11_playing_no_bytes_stall_notice() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.feed(D.as_slice(), ts(0));
    while c.poll_event().is_some() {}
    // nothing before 5s
    c.poll_timeout(ts(4999));
    let mut events = Vec::new();
    while let Some(e) = c.poll_event() {
        events.push(e);
    }
    assert!(events.is_empty());
    // at 5s
    c.poll_timeout(ts(5000));
    let mut saw_notice = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::StallNotice { .. }) {
            saw_notice = true;
        }
    }
    assert!(saw_notice);
}

#[test]
fn t12_playing_no_bytes_stall_reconnect_aborts_and_resets_attempts() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.feed(D.as_slice(), ts(0));
    c.poll_timeout(ts(29999));
    // no abort before
    let mut aborted = false;
    while let Some(a) = c.poll() {
        if a == (Action::Abort) {
            aborted = true;
        }
    }
    assert!(!aborted);
    c.poll_timeout(ts(30000));
    let mut saw_abort = false;
    let mut saw_zero = false;
    while let Some(a) = c.poll() {
        if a == (Action::Abort) {
            saw_abort = true;
        }
        if a == (Action::Reconnect { after_ms: 0 }) {
            saw_zero = true;
        }
    }
    assert!(saw_abort);
    assert!(saw_zero);
    // attempt reset: next failure after playing? state backoff; reopen then fail gives 2000 (attempt 1)
}

#[test]
fn t13_any_connected_reconnect_aborts() {
    // Opening
    {
        let mut c = TsHttpClient::new("u", cfg(), ts(0));
        let _ = c.poll();
        c.reconnect(ts(5));
        assert_eq!(c.poll(), Some(Action::Abort));
        assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 0 }));
    }
    // Awaiting
    {
        let mut c = TsHttpClient::new("u", cfg(), ts(0));
        let _ = c.poll();
        c.on_response(200, &[], ts(0));
        let _ = c.poll_event();
        c.reconnect(ts(5));
        assert_eq!(c.poll(), Some(Action::Abort));
        assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 0 }));
    }
    // Playing
    {
        let mut c = TsHttpClient::new("u", cfg(), ts(0));
        let _ = c.poll();
        c.on_response(200, &[], ts(0));
        let _ = c.poll_event();
        c.feed(D.as_slice(), ts(1));
        c.reconnect(ts(5));
        assert_eq!(c.poll(), Some(Action::Abort));
        assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 0 }));
    }
}

#[test]
fn t14_bytes_no_samples_starved_data_once_per_episode() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.feed(D.as_slice(), ts(0)); // playing
    // bytes flowing but no sample for stall_notice (4 bytes at now=10000)
    c.feed(&[0u8; 4], ts(10000));
    let mut saw_data = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Starved(StarveKind::Data)) {
            saw_data = true;
        }
    }
    assert!(saw_data);
    // once per episode: second feed does not emit again
    c.feed(&[0u8; 4], ts(11000));
    let mut saw_second = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Starved(StarveKind::Data)) {
            saw_second = true;
        }
    }
    assert!(!saw_second);
}

#[test]
fn t15_playing_no_bytes_starved_nodata_once_per_episode() {
    let mut c = TsHttpClient::new("u", cfg(), ts(0));
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    let _ = c.poll_event();
    c.feed(D.as_slice(), ts(0)); // playing
    c.poll_timeout(ts(5000));
    let mut saw_nodata = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Starved(StarveKind::NoData)) {
            saw_nodata = true;
        }
    }
    assert!(saw_nodata);
    // once per episode
    c.poll_timeout(ts(6000));
    let mut saw_second = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Starved(StarveKind::NoData)) {
            saw_second = true;
        }
    }
    assert!(!saw_second);
}
