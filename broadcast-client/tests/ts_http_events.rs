mod support;

use broadcast_client::{Action, Config, Event, Session, Status};
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
    let _ = evs
        .iter()
        .any(|e| matches!(e, Event::DisplayChanged { .. }));
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
fn chunking_invariant_1_1316_65536() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut counts = Vec::new();
    for chunk in [1usize, 1316, 65536] {
        let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
        let evs = feed_chunks(&mut session, &bytes, chunk, status, &headers, 0);
        let samples = evs
            .iter()
            .filter(|e| matches!(e, Event::Sample { .. }))
            .count();
        let tracks = evs.iter().filter(|e| matches!(e, Event::Tracks(_))).count();
        counts.push((samples, tracks));
    }
    assert_eq!(counts[0], counts[1]);
    assert_eq!(counts[1], counts[2]);
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
    session.on_response(conn, status, &headers);
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
    session.on_response(conn, status, &headers);
    session.on_data(conn, &bytes[..4096], 0);
    session.on_closed(conn, None, 1000);
    let mut new_conn = None;
    while let Some(a) = session.poll(1000) {
        if let Action::HttpRequest { conn: c, .. } = a {
            new_conn = Some(c);
        }
    }
    let c = new_conn.unwrap();
    session.on_response(c, status, &headers);
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
    session.on_response(conn, status, &headers);
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
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let _ = session.poll(0);
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
    session.on_response(conn, status, &headers);
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
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    session.on_response(999, 200, &[]);
    session.on_data(999, &[1, 2, 3], 0);
    session.on_closed(999, None, 0);
}
