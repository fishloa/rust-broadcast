use crate::support;
use url::Url;

use broadcast_client::{
    Action, Config, ConnId, Event, Session, StarveKind, Status, TransportError,
};
use broadcast_common::stage::Timestamp;
use support::*;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

#[test]
fn tracks_precede_first_sample() {
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
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
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let evs = feed_chunks(&mut session, &bytes, 1316, status, &headers, 0);
    let tracks_count = evs.iter().filter(|e| matches!(e, Event::Tracks(_))).count();
    assert!(tracks_count >= 1);
}

#[test]
fn display_changed_keeps_position_between_samples() {
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
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
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
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
    session.on_data(conn, &bytes, ts(0));
    let _ = session.poll(ts(10_000_000));
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        println!("EV: {:?}", e);
        evs.push(e);
    }
    session.on_data(conn, &[0x47], ts(10_000_001));
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
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut config = Config::default();
    config.ts_http.min_reconnect_interval = std::time::Duration::ZERO;
    let mut session =
        Session::open(Url::parse("http://example/live.ts").unwrap(), config, ts(0)).unwrap();
    let conn = match session.poll(ts(0)).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!(),
    };
    session.on_response(conn, status, &headers, ts(0));
    session.on_data(conn, &bytes[..4096], ts(0));
    session.on_closed(conn, None, ts(1000));
    let mut new_conn = None;
    while let Some(a) = session.poll(ts(1000)) {
        if let Action::HttpRequest { conn: c, .. } = a {
            new_conn = Some(c);
        }
    }
    let c = new_conn.unwrap();
    session.on_response(c, status, &headers, ts(1000));
    session.on_data(c, &bytes[..65536], ts(2000));
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        println!("EV2: {:?}", e);
        evs.push(e);
    }
    assert!(evs.iter().any(|e| matches!(e, Event::Discontinuity)));
}

#[test]
fn not_found_status_and_keeps_retrying_at_cap() {
    let (status, headers) = missing_headers!();
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
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        evs.push(e);
    }
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::Status(Status::NotFound)))
    );
    let mut saw_wake = false;
    while let Some(a) = session.poll(ts(0)) {
        if matches!(a, Action::WakeAt(_)) {
            saw_wake = true;
        }
    }
    assert!(saw_wake);
}

#[test]
fn starved_event_passthrough() {
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
        other => panic!("{other:?}"),
    };
    session.on_response(conn, status, &headers, ts(0));
    session.on_data(conn, &bytes, ts(0));
    while session.next_event().is_some() {}
    let _ = session.poll(ts(10_000_000));
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

    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let conn = match session.poll(ts(0)).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    session.on_response(
        conn,
        http::StatusCode::from_u16(200).expect("a test names a real status"),
        &crate::support::headers(&[]),
        ts(0),
    );
    let mut null_packet = vec![0xFF; 188];
    null_packet[..4].copy_from_slice(&[0x47, 0x1F, 0xFF, 0x10]);
    session.on_data(conn, &null_packet, ts(10_000_000));
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
    let err = Session::open(
        Url::parse("ftp://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        broadcast_client::Error::UnsupportedScheme { .. }
    ));
}

#[test]
fn close_is_terminal_and_emits_close_once() {
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
    session.close(ts(0));
    let mut closes = 0;
    while let Some(a) = session.poll(ts(0)) {
        if matches!(a, Action::Close { .. }) {
            closes += 1;
        }
    }
    assert_eq!(closes, 1);
    assert!(session.poll(ts(0)).is_none());
    assert!(session.next_event().is_none());
}

#[test]
fn unknown_conn_callbacks_ignored() {
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
        other => panic!("{other:?}"),
    };
    let unknown = ConnId::new(conn.0.wrapping_add(998));
    session.on_response(
        unknown,
        http::StatusCode::from_u16(200).expect("a test names a real status"),
        &crate::support::headers(&[]),
        ts(0),
    );
    session.on_data(unknown, &bytes[..4096], ts(0));
    session.on_closed(unknown, None, ts(1_000));
    session.on_closed(
        unknown,
        Some(TransportError::Other {
            reason: "reset".to_string(),
        }),
        ts(2_000),
    );
    let mut actions = Vec::new();
    while let Some(a) = session.poll(ts(3_000)) {
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

    session.on_response(conn, status, &headers, ts(3_000));
    for chunk in bytes.chunks(65_536).take(8) {
        session.on_data(conn, chunk, ts(4_000));
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

/// The demuxer's tail (the last PES of every PID) is flushed when the
/// connection closes and handed to the host; the controller learns those ends
/// in the same `on_closed`, so it never judges the lead short of what the host
/// holds.
#[test]
fn media_flushed_at_connection_close_reaches_the_controller_in_the_same_call() {
    use broadcast_client::BufferInput;
    use std::collections::BTreeMap;
    use support::replay::{bitrate_bps, open_live};

    let bytes = test_fixtures::read("ts/h264-aac-40s.ts");
    let bps = bitrate_bps(&bytes);
    let mut config = Config::default();
    config.record_buffer_inputs = true;
    let (mut s, conn) = open_live(config);
    let mut timescale: BTreeMap<u32, u32> = BTreeMap::new();
    let mut host_end: BTreeMap<u32, u64> = BTreeMap::new();
    let mut reported: BTreeMap<u32, u64> = BTreeMap::new();
    let mut now = 0u64;
    let mut drain = |s: &mut broadcast_client::Session,
                     host_end: &mut BTreeMap<u32, u64>,
                     reported: &mut BTreeMap<u32, u64>| {
        while let Some(e) = s.next_event() {
            match e {
                Event::Tracks(specs) => {
                    for t in specs {
                        timescale.insert(t.track_id.value(), t.timescale.get());
                    }
                }
                Event::Sample { track, sample, .. } => {
                    let ts = u64::from(timescale[&track.value()]);
                    let end = u64::try_from(
                        sample.timing.and_then(|t| t.pts()).unwrap()
                            + i64::from(sample.timing.and_then(|t| t.duration()).unwrap_or(0)),
                    )
                    .unwrap();
                    let e27 = end * 27_000_000 / ts;
                    let slot = host_end.entry(track.value()).or_insert(0);
                    *slot = (*slot).max(e27);
                }
                _ => {}
            }
        }
        for i in s.drain_buffer_inputs() {
            if let BufferInput::TrackMedia { track, end, .. } = i {
                let slot = reported.entry(track).or_insert(0);
                *slot = (*slot).max(end.ticks());
            }
        }
    };
    for part in bytes.chunks(188 * 7) {
        s.on_data(conn, part, ts(now));
        drain(&mut s, &mut host_end, &mut reported);
        now += part.len() as u64 * 8 * 1_000_000 / bps;
    }
    let before_close = reported.clone();
    s.on_closed(conn, None, ts(now));
    drain(&mut s, &mut host_end, &mut reported);
    let mut short = Vec::new();
    for (track, rep) in &reported {
        let host = host_end[track];
        if host > *rep {
            short.push((*track, (host - rep) / 27_000));
        }
    }
    assert!(
        short.is_empty(),
        "controller told less than the host holds after close, (track, ms short): {short:?}; reported before close {before_close:?}"
    );
}
