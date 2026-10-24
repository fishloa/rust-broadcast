mod support;

use broadcast_client::{Action, Config, Event, Session, Status};
use support::*;

#[test]
fn session_finite_file_ends_without_reconnect() {
    let bytes = read_tf1_bytes();
    let (status0, base_headers) = read_tf1_headers();
    let mut headers = base_headers;
    headers.push(("Content-Length".to_string(), bytes.len().to_string()));
    let status = status0;

    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, 0);
    for chunk in bytes.chunks(1316) {
        session.on_data(conn, chunk, 0);
    }
    session.on_closed(conn, None, 1000);

    // Drain events collected so far.
    let mut ended = 0usize;
    while let Some(e) = session.next_event() {
        if matches!(e, Event::Status(Status::Ended)) {
            ended += 1;
        }
    }

    // 10 minutes of poll(now_us): no further HttpRequest.
    let mut http_after = 0usize;
    let mut a = session.poll(600_000_000);
    while a.is_some() {
        if matches!(a, Some(Action::HttpRequest { .. })) {
            http_after += 1;
        }
        a = session.poll(600_000_000);
    }
    assert_eq!(ended, 1);
    assert_eq!(http_after, 0);
}

#[test]
fn session_live_unchanged_reconnects_on_close() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, 0);
    session.on_data(conn, &bytes[..4096], 0);
    session.on_closed(conn, None, 1000);

    let mut saw_reconnect = false;
    let mut a = session.poll(1000);
    while a.is_some() {
        if matches!(a, Some(Action::WakeAt(_))) || matches!(a, Some(Action::HttpRequest { .. })) {
            saw_reconnect = true;
        }
        a = session.poll(1000);
    }
    assert!(saw_reconnect);
}

#[test]
fn session_finite_event_emitted_once() {
    let bytes = read_tf1_bytes();
    let (status0, base_headers) = read_tf1_headers();
    let mut headers = base_headers;
    headers.push(("Content-Length".to_string(), bytes.len().to_string()));
    let status = status0;

    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let evs = feed_chunks(&mut session, &bytes, 1316, status, &headers, 0);
    let finite_count = evs
        .iter()
        .filter(|e| matches!(e, Event::Finite { .. }))
        .count();
    assert_eq!(finite_count, 1);
}

#[test]
fn session_reconnect_after_ended_restarts() {
    let bytes = read_tf1_bytes();
    let (status0, base_headers) = read_tf1_headers();
    let mut headers = base_headers;
    headers.push(("Content-Length".to_string(), bytes.len().to_string()));
    let status = status0;

    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, 0);
    for chunk in bytes.chunks(1316) {
        session.on_data(conn, chunk, 0);
    }
    session.on_closed(conn, None, 1000);
    while session.next_event().is_some() {}

    session.reconnect(2000);
    let mut reopened = false;
    let mut a = session.poll(2000);
    while a.is_some() {
        if matches!(a, Some(Action::HttpRequest { .. })) {
            reopened = true;
        }
        a = session.poll(2000);
    }
    assert!(reopened);
}

#[test]
fn session_206_is_finite() {
    let bytes = read_tf1_bytes();
    let headers = vec![(
        "Content-Range".to_string(),
        format!("bytes 0-{}/{}", bytes.len() - 1, bytes.len()),
    )];
    let status = 206u16;

    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let evs = feed_chunks(&mut session, &bytes, 1316, status, &headers, 0);
    let finite_count = evs
        .iter()
        .filter(|e| matches!(e, Event::Finite { .. }))
        .count();
    assert_eq!(finite_count, 1);
}

#[test]
fn truncated_finite_transfer_reconnects() {
    let bytes = read_tf1_bytes();
    let (status0, base_headers) = read_tf1_headers();
    let mut headers = base_headers;
    headers.push(("Content-Length".to_string(), bytes.len().to_string()));
    let status = status0;

    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, 0);
    session.on_data(conn, &bytes[..4096], 0);
    session.on_closed(conn, None, 1000);

    let mut wake = false;
    let mut a = session.poll(1000);
    while a.is_some() {
        if matches!(a, Some(Action::WakeAt(_))) || matches!(a, Some(Action::HttpRequest { .. })) {
            wake = true;
        }
        a = session.poll(1000);
    }
    assert!(wake);
}
