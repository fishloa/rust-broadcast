use tshttp_runtime::{Action, Config, Event, TsHttpClient};

const D: &[u8] = include_bytes!("../../../fixtures/ts/h264_aac.ts");

fn cfg() -> Config {
    Config::default()
}

#[test]
fn finite_content_length_eof_is_ended_not_reconnect() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(
        200,
        &[("Content-Length".to_string(), D.len().to_string())],
        0,
    );
    assert!(matches!(c.poll_event(), Some(Event::Connected)));
    assert!(matches!(
        c.poll_event(),
        Some(Event::Finite { expected_bytes }) if expected_bytes == D.len() as u64
    ));
    for chunk in D.chunks(1316) {
        c.feed(chunk, 0);
    }
    c.on_eof(1);
    let mut ended = false;
    let mut reconnect = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Ended) {
            ended = true;
        }
    }
    while let Some(a) = c.poll() {
        if matches!(a, Action::Reconnect { .. }) {
            reconnect = true;
        }
    }
    assert!(ended);
    assert!(!reconnect);
}

#[test]
fn chunked_without_length_stays_live() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(
        200,
        &[("Transfer-Encoding".to_string(), "chunked".to_string())],
        0,
    );
    for chunk in D.chunks(1316) {
        c.feed(chunk, 0);
    }
    c.on_eof(60_000);
    let mut reconnect0 = false;
    let mut ended = false;
    while let Some(a) = c.poll() {
        if matches!(a, Action::Reconnect { after_ms: 0 }) {
            reconnect0 = true;
        }
    }
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Ended) {
            ended = true;
        }
    }
    assert!(reconnect0);
    assert!(!ended);
}

#[test]
fn no_content_length_http2_style_stays_live() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(200, &[], 0);
    for chunk in D.chunks(1316) {
        c.feed(chunk, 0);
    }
    c.on_eof(60_000);
    let mut reconnect0 = false;
    while let Some(a) = c.poll() {
        if matches!(a, Action::Reconnect { after_ms: 0 }) {
            reconnect0 = true;
        }
    }
    assert!(reconnect0);
}

#[test]
fn status_206_with_content_range_is_finite() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(
        206,
        &[("Content-Range".to_string(), "bytes 0-9/10".to_string())],
        0,
    );
    assert!(matches!(c.poll_event(), Some(Event::Connected)));
    assert!(matches!(
        c.poll_event(),
        Some(Event::Finite { expected_bytes: 10 })
    ));
}

#[test]
fn bad_or_negative_content_length_is_live() {
    for val in ["-1", "abc", "1.5"] {
        let mut c = TsHttpClient::new("u", cfg(), 0);
        let _ = c.poll();
        c.on_response(200, &[("Content-Length".to_string(), val.to_string())], 0);
        let mut finite = false;
        while let Some(e) = c.poll_event() {
            if matches!(e, Event::Finite { .. }) {
                finite = true;
            }
        }
        assert!(!finite, "val={val}");
    }
}

#[test]
fn content_length_header_case_insensitive() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(200, &[("content-length".to_string(), "5".to_string())], 0);
    let mut finite = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Finite { expected_bytes: 5 }) {
            finite = true;
        }
    }
    assert!(finite);
}

#[test]
fn finite_eof_before_all_bytes_is_failure_backoff() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(
        200,
        &[("Content-Length".to_string(), D.len().to_string())],
        0,
    );
    let _ = c.poll_event();
    let _ = c.poll_event();
    c.feed(&D[..1000], 0);
    c.on_eof(1);
    let mut reconnecting = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Reconnecting { attempt: 1 }) {
            reconnecting = true;
        }
    }
    assert!(reconnecting);
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 2000 }));
}

#[test]
fn timers_stop_after_all_bytes_received() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(
        200,
        &[("Content-Length".to_string(), D.len().to_string())],
        0,
    );
    let _ = c.poll_event();
    let _ = c.poll_event();
    for chunk in D.chunks(1316) {
        c.feed(chunk, 0);
    }
    while c.poll_event().is_some() {}
    c.poll_timeout(60_000);
    // no StallNotice/Starved/Abort
    let mut bad = false;
    while let Some(e) = c.poll_event() {
        if matches!(
            e,
            Event::StallNotice { .. } | Event::Starved(_) | Event::Reconnecting { .. }
        ) {
            bad = true;
        }
    }
    while let Some(a) = c.poll() {
        if matches!(a, Action::Abort) {
            bad = true;
        }
    }
    assert!(!bad);
}

#[test]
fn ended_is_terminal_no_actions_forever() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(
        200,
        &[("Content-Length".to_string(), D.len().to_string())],
        0,
    );
    let _ = c.poll_event();
    let _ = c.poll_event();
    for chunk in D.chunks(1316) {
        c.feed(chunk, 0);
    }
    c.on_eof(1);
    while c.poll_event().is_some() {}
    while c.poll().is_some() {}
    c.poll_timeout(10 * 60 * 1000);
    assert!(c.poll_event().is_none());
    assert!(c.poll().is_none());
    c.feed(&[1, 2, 3], 2);
    c.on_eof(3);
    assert!(c.poll_event().is_none());
    assert!(c.poll().is_none());
}

#[test]
fn caller_reconnect_from_ended_reopens_and_resets_attempts() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(
        200,
        &[("Content-Length".to_string(), D.len().to_string())],
        0,
    );
    let _ = c.poll_event();
    let _ = c.poll_event();
    for chunk in D.chunks(1316) {
        c.feed(chunk, 0);
    }
    c.on_eof(1);
    while c.poll_event().is_some() {}
    while c.poll().is_some() {}
    c.reconnect(5);
    assert_eq!(c.poll(), Some(Action::Abort));
    assert_eq!(c.poll(), Some(Action::Reconnect { after_ms: 0 }));
    c.poll_timeout(5);
    assert_eq!(
        c.poll(),
        Some(Action::Open {
            url: "u".to_string()
        })
    );
}

#[test]
fn finite_flushes_demux_before_ended() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(
        200,
        &[("Content-Length".to_string(), D.len().to_string())],
        0,
    );
    let _ = c.poll_event();
    let _ = c.poll_event();
    for chunk in D.chunks(1316) {
        c.feed(chunk, 0);
    }
    c.on_eof(1);
    let mut evs = Vec::new();
    while let Some(e) = c.poll_event() {
        evs.push(e);
    }
    let last = evs.last().unwrap();
    assert!(matches!(last, Event::Ended));
    assert!(evs.iter().any(|e| matches!(e, Event::Demux(_))));
}

#[test]
fn finite_event_follows_connected() {
    let mut c = TsHttpClient::new("u", cfg(), 0);
    let _ = c.poll();
    c.on_response(200, &[("Content-Length".to_string(), "5".to_string())], 0);
    assert!(matches!(c.poll_event(), Some(Event::Connected)));
    assert!(matches!(
        c.poll_event(),
        Some(Event::Finite { expected_bytes: 5 })
    ));
}
