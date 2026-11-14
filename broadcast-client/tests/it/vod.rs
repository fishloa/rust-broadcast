use crate::support;
use url::Url;

use broadcast_client::{Action, Config, Event, Session, Status};
use broadcast_common::stage::Timestamp;
use support::*;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

#[test]
fn session_finite_file_ends_without_reconnect() {
    let bytes = tf1_bytes!();
    let (status0, base_headers) = tf1_headers!();
    let mut headers = base_headers;
    headers.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&bytes.len().to_string()).expect("a test writes a length"),
    );
    let status = status0;

    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let conn = match session.poll(ts(0)).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, ts(0));
    for chunk in bytes.chunks(1316) {
        session.on_data(conn, chunk, ts(0));
    }
    session.on_closed(conn, None, ts(1000));

    // Drain events collected so far.
    let mut ended = 0usize;
    while let Some(e) = session.next_event() {
        if matches!(e, Event::Status(Status::Ended)) {
            ended += 1;
        }
    }

    // 10 minutes of poll(now_us): no further HttpRequest.
    let mut http_after = 0usize;
    let mut a = session.poll(ts(600_000_000));
    while a.is_some() {
        if matches!(a, Some(Action::HttpRequest { .. })) {
            http_after += 1;
        }
        a = session.poll(ts(600_000_000));
    }
    assert_eq!(ended, 1);
    assert_eq!(http_after, 0);
}

#[test]
fn session_live_unchanged_reconnects_on_close() {
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let conn = match session.poll(ts(0)).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, ts(0));
    session.on_data(conn, &bytes[..4096], ts(0));
    session.on_closed(conn, None, ts(1000));

    let mut saw_reconnect = false;
    let mut a = session.poll(ts(1000));
    while a.is_some() {
        if matches!(a, Some(Action::WakeAt(_))) || matches!(a, Some(Action::HttpRequest { .. })) {
            saw_reconnect = true;
        }
        a = session.poll(ts(1000));
    }
    assert!(saw_reconnect);
}

#[test]
fn session_finite_event_emitted_once() {
    let bytes = tf1_bytes!();
    let (status0, base_headers) = tf1_headers!();
    let mut headers = base_headers;
    headers.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&bytes.len().to_string()).expect("a test writes a length"),
    );
    let status = status0;

    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let evs = feed_chunks(&mut session, &bytes, 1316, status, &headers, 0);
    let finite_count = evs
        .iter()
        .filter(|e| matches!(e, Event::Finite { .. }))
        .count();
    assert_eq!(finite_count, 1);
}

#[test]
fn session_reconnect_after_ended_restarts() {
    let bytes = tf1_bytes!();
    let (status0, base_headers) = tf1_headers!();
    let mut headers = base_headers;
    headers.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&bytes.len().to_string()).expect("a test writes a length"),
    );
    let status = status0;

    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let conn = match session.poll(ts(0)).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, ts(0));
    for chunk in bytes.chunks(1316) {
        session.on_data(conn, chunk, ts(0));
    }
    session.on_closed(conn, None, ts(1000));
    while session.next_event().is_some() {}

    session.reconnect(ts(2000));
    let mut reopened = false;
    let mut a = session.poll(ts(2000));
    while a.is_some() {
        if matches!(a, Some(Action::HttpRequest { .. })) {
            reopened = true;
        }
        a = session.poll(ts(2000));
    }
    assert!(reopened);
}

#[test]
fn session_206_is_finite() {
    let bytes = tf1_bytes!();
    let headers = crate::support::headers(&[(
        "Content-Range",
        &format!("bytes 0-{}/{}", bytes.len() - 1, bytes.len()),
    )]);
    let status = http::StatusCode::PARTIAL_CONTENT;

    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let evs = feed_chunks(&mut session, &bytes, 1316, status, &headers, 0);
    let finite_count = evs
        .iter()
        .filter(|e| matches!(e, Event::Finite { .. }))
        .count();
    assert_eq!(finite_count, 1);
}

#[test]
fn truncated_finite_transfer_reconnects() {
    let bytes = tf1_bytes!();
    let (status0, base_headers) = tf1_headers!();
    let mut headers = base_headers;
    headers.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&bytes.len().to_string()).expect("a test writes a length"),
    );
    let status = status0;

    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let conn = match session.poll(ts(0)).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, ts(0));
    session.on_data(conn, &bytes[..4096], ts(0));
    session.on_closed(conn, None, ts(1000));

    let mut wake = false;
    let mut a = session.poll(ts(1000));
    while a.is_some() {
        if matches!(a, Some(Action::WakeAt(_))) || matches!(a, Some(Action::HttpRequest { .. })) {
            wake = true;
        }
        a = session.poll(ts(1000));
    }
    assert!(wake);
}
