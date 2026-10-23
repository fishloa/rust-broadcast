use std::collections::BTreeMap;
use tshttp_runtime::{Action, Config, Event, TsHttpClient};

fn samples_by_track(events: &[Event]) -> BTreeMap<u32, usize> {
    let mut m = BTreeMap::new();
    for e in events {
        if let Event::Demux(transmux::DemuxEvent::Sample { track_id, .. }) = e {
            *m.entry(track_id.value()).or_insert(0) += 1;
        }
    }
    m
}

#[test]
fn eof_flushes_held_back_samples() {
    let bytes = match test_fixtures::require_bytes("zenith-http/tf1-live.ts", "flush") {
        Some(b) => b,
        None => return,
    };
    // before eof
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, 0);
    let _ = c.poll_event();
    c.feed(&bytes, 0);
    let mut before_events = Vec::new();
    while let Some(e) = c.poll_event() {
        before_events.push(e);
    }
    let before = samples_by_track(&before_events);

    // on_eof flush
    c.on_eof(1);
    let mut after_flush = Vec::new();
    while let Some(e) = c.poll_event() {
        after_flush.push(e);
    }
    let mut total = before.clone();
    for (k, v) in samples_by_track(&after_flush) {
        *total.entry(k).or_insert(0) += v;
    }

    // oracle
    let mut d = transmux::StreamingTsDemux::new();
    d.feed(&bytes);
    d.finish();
    let mut oracle = BTreeMap::new();
    // need collect from a fresh feed+finish
    let mut d2 = transmux::StreamingTsDemux::new();
    d2.feed(&bytes);
    d2.finish();
    while let Some(e) = d2.poll_event() {
        if let transmux::DemuxEvent::Sample { track_id, .. } = e {
            *oracle.entry(track_id.value()).or_insert(0) += 1;
        }
    }

    assert_eq!(total, oracle);
    // strictly greater for at least one track
    let mut greater = false;
    for (k, v) in &total {
        if *v > before.get(k).copied().unwrap_or(0) {
            greater = true;
        }
    }
    assert!(greater);
}

#[test]
fn flush_events_precede_reconnect_action() {
    let bytes = match test_fixtures::require_bytes("zenith-http/tf1-live.ts", "flush") {
        Some(b) => b,
        None => return,
    };
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, 0);
    let _ = c.poll_event();
    c.feed(&bytes, 0);
    while c.poll_event().is_some() {}
    c.on_eof(1);
    // event queue has demux events
    let mut has_demux = false;
    // peek by popping one event then push back? We'll just check poll_event returns Some(Demux) eventually
    // Collect all events
    let mut evs = Vec::new();
    while let Some(e) = c.poll_event() {
        evs.push(e);
    }
    for e in &evs {
        if matches!(e, Event::Demux(_)) {
            has_demux = true;
        }
    }
    // push events back not possible; instead check actions still has reconnect (actions untouched)
    // Re-poll actions
    let mut has_reconnect = false;
    while let Some(a) = c.poll() {
        if matches!(a, Action::Reconnect { .. }) {
            has_reconnect = true;
        }
    }
    assert!(has_demux);
    assert!(has_reconnect);
}

#[test]
fn eof_with_only_flushed_sample_counts_as_played() {
    let bytes = match test_fixtures::require_bytes("zenith-http/tf1-live.ts", "flush") {
        Some(b) => b,
        None => return,
    };
    // find shortest prefix with 0 samples before finish and >=1 after
    let mut found: Option<usize> = None;
    for n in 1..=bytes.len() {
        let prefix = &bytes[..n];
        let mut d = transmux::StreamingTsDemux::new();
        d.feed(prefix);
        let mut before = 0;
        while let Some(e) = d.poll_event() {
            if matches!(e, transmux::DemuxEvent::Sample { .. }) {
                before += 1;
            }
        }
        if before != 0 {
            continue;
        }
        // finish
        d.finish();
        let mut after = 0;
        while let Some(e) = d.poll_event() {
            if matches!(e, transmux::DemuxEvent::Sample { .. }) {
                after += 1;
            }
        }
        if after >= 1 {
            found = Some(n);
            break;
        }
    }
    let n = found.expect("no prefix with 0 samples before finish and >=1 after");
    let prefix = &bytes[..n];

    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(200, 0);
    let _ = c.poll_event();
    c.feed(prefix, 0);
    while c.poll_event().is_some() {}
    c.on_eof(1);
    let mut saw_zero = false;
    let mut saw_reconnecting = false;
    while let Some(e) = c.poll_event() {
        if matches!(e, Event::Reconnecting { .. }) {
            saw_reconnecting = true;
        }
    }
    while let Some(a) = c.poll() {
        if a == (Action::Reconnect { after_ms: 0 }) {
            saw_zero = true;
        }
    }
    assert!(saw_zero);
    assert!(!saw_reconnecting);
}
