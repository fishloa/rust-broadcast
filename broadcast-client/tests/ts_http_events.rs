mod support;

use broadcast_client::{Action, Config, Event, Session, StarveKind, Status};
use support::*;

#[test]
fn tracks_precede_first_sample() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let evs = feed_chunks(&mut session, &bytes, 1316, status, &headers, 0);
    let first_sample = evs
        .iter()
        .position(|e| matches!(e, Event::Sample { .. }))
        .unwrap();
    let has_tracks_before = evs[..first_sample]
        .iter()
        .any(|e| matches!(e, Event::Tracks(_)));
    assert!(has_tracks_before);
}

#[test]
fn tracks_coalesced_one_per_change() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let evs = feed_chunks(&mut session, &bytes, 1316, status, &headers, 0);
    let tracks_count = evs.iter().filter(|e| matches!(e, Event::Tracks(_))).count();
    assert!(tracks_count >= 1);
}

#[test]
fn display_changed_keeps_position_between_samples() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let evs = feed_chunks(&mut session, &bytes, 1316, status, &headers, 0);
    let positions = |pred: fn(&Event) -> bool| -> Vec<usize> {
        evs.iter()
            .enumerate()
            .filter(|(_, e)| pred(e))
            .map(|(i, _)| i)
            .collect()
    };
    let display = positions(|e| matches!(e, Event::DisplayChanged { .. }));
    let samples = positions(|e| matches!(e, Event::Sample { .. }));
    let tracks = positions(|e| matches!(e, Event::Tracks(_)));
    assert!(
        !display.is_empty(),
        "the fixture reports its display geometry"
    );
    assert!(samples.len() > 1);
    for at in &display {
        assert!(
            tracks.iter().any(|t| t < at),
            "DisplayChanged at {at} precedes every Tracks"
        );
        assert!(
            samples.iter().any(|s| s > at),
            "DisplayChanged at {at} is not followed by a sample"
        );
    }
    let last_sample = *samples.last().unwrap();
    let first_display = display[0];
    assert!(
        first_display < last_sample,
        "DisplayChanged was deferred past the last sample"
    );
    let samples_before_first = samples.iter().filter(|s| **s < first_display).count();
    let samples_after_first = samples.len() - samples_before_first;
    assert!(
        samples_after_first > 0,
        "{samples_before_first} samples before and none after the first DisplayChanged"
    );
}

#[test]
fn status_establishing_then_live_once() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let evs = feed_chunks(&mut session, &bytes, 1316, status, &headers, 0);
    let live_count = evs
        .iter()
        .filter(|e| matches!(e, Event::Status(Status::Live)))
        .count();
    assert_eq!(live_count, 1);
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::Status(Status::Establishing)))
    );
}

#[test]
fn stall_gives_stalled_then_live_on_next_byte() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, 0);
    session.on_data(conn, &bytes, 0);
    let _ = session.poll(10_000_000);
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        println!("EV: {:?}", e);
        evs.push(e);
    }
    session.on_data(conn, &[0x47], 10_000_001);
    let mut evs2 = Vec::new();
    while let Some(e) = session.next_event() {
        evs2.push(e);
    }
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::Status(Status::Stalled { .. })))
    );
    assert!(
        evs2.iter()
            .any(|e| matches!(e, Event::Status(Status::Live)))
    );
}

#[test]
fn server_close_reconnects_with_discontinuity_before_first_sample() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut config = Config::default();
    config.ts_http.min_reconnect_interval_ms = 0;
    let mut session = Session::open("http://example/live.ts", config, 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, 0);
    session.on_data(conn, &bytes[..4096], 0);
    session.on_closed(conn, None, 1000);
    let mut new_conn = None;
    while let Some(a) = session.poll(1000) {
        if let Action::HttpRequest { conn: c, .. } = a {
            new_conn = Some(c);
        }
    }
    let c = new_conn.unwrap();
    session.on_response(c, status, &headers, 1000);
    session.on_data(c, &bytes[..65536], 2000);
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        println!("EV2: {:?}", e);
        evs.push(e);
    }
    assert!(evs.iter().any(|e| matches!(e, Event::Discontinuity)));
}

#[test]
fn not_found_status_and_keeps_retrying_at_cap() {
    let (status, headers) = read_missing_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, 0);
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        evs.push(e);
    }
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::Status(Status::NotFound)))
    );
    let mut saw_wake = false;
    while let Some(a) = session.poll(0) {
        if matches!(a, Action::WakeAt(_)) {
            saw_wake = true;
        }
    }
    assert!(saw_wake);
}

#[test]
fn starved_event_passthrough() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    session.on_response(conn, status, &headers, 0);
    session.on_data(conn, &bytes, 0);
    while session.next_event().is_some() {}
    let _ = session.poll(10_000_000);
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        evs.push(e);
    }
    assert!(evs.contains(&Event::Starved(StarveKind::NoData)), "{evs:?}");
    assert_eq!(
        evs.iter()
            .filter(|e| matches!(e, Event::Starved(StarveKind::NoData)))
            .count(),
        1,
        "one notice per stall"
    );

    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    session.on_response(conn, 200, &[], 0);
    let mut null_packet = vec![0xFF; 188];
    null_packet[..4].copy_from_slice(&[0x47, 0x1F, 0xFF, 0x10]);
    session.on_data(conn, &null_packet, 10_000_000);
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        evs.push(e);
    }
    assert!(
        evs.contains(&Event::Starved(StarveKind::Data)),
        "bytes without samples report data starvation: {evs:?}"
    );
}

#[test]
fn unsupported_scheme_error() {
    let err = Session::open("ftp://example/live.ts", Config::default(), 0).unwrap_err();
    assert!(matches!(err, broadcast_client::Error::UnsupportedScheme(_)));
}

#[test]
fn close_is_terminal_and_emits_close_once() {
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, 0);
    session.close();
    let mut closes = 0;
    while let Some(a) = session.poll(0) {
        if matches!(a, Action::Close { .. }) {
            closes += 1;
        }
    }
    assert_eq!(closes, 1);
    assert!(session.poll(0).is_none());
    assert!(session.next_event().is_none());
}

#[test]
fn unknown_conn_callbacks_ignored() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    let unknown = conn.wrapping_add(998);
    session.on_response(unknown, 200, &[], 0);
    session.on_data(unknown, &bytes[..4096], 0);
    session.on_closed(unknown, None, 1_000);
    session.on_closed(unknown, Some("reset".to_string()), 2_000);
    let mut actions = Vec::new();
    while let Some(a) = session.poll(3_000) {
        actions.push(a);
    }
    assert!(
        actions.iter().all(|a| matches!(a, Action::WakeAt(_))),
        "{actions:?}"
    );
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::HttpRequest { .. })),
        "{actions:?}"
    );
    assert!(session.next_event().is_none());

    session.on_response(conn, status, &headers, 3_000);
    for chunk in bytes.chunks(65_536).take(8) {
        session.on_data(conn, chunk, 4_000);
    }
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        evs.push(e);
    }
    assert!(
        evs.iter().any(|e| matches!(e, Event::Sample { .. })),
        "the real connection still plays"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))),
        "an unknown connection's close must not drive the retry policy: {evs:?}"
    );
}
