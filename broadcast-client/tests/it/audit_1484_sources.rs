use crate::support;
use url::Url;

use std::collections::HashMap;

use broadcast_client::{
    Action, BufferEvent, Config, ConnId, Credentials, ErrorKind, Event, Session, SourceKind,
    Status, TrackId,
};
use broadcast_common::stage::Timestamp;
use support::hls_host::*;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}
use support::renditions::{
    AUDIO_DIR, AUDIO_PLAYLIST_URL, MASTER_URL, VIDEO_DIR, VIDEO_PLAYLIST_URL, fixture,
};
use support::rtsp_host::{
    BASE_URL, Host as RtspHost, MediaFixture, SEC as RTSP_SEC, Server, connects as rtsp_connects,
    wakes as rtsp_wakes,
};

const RTSP_URL: &str = "rtsp://cam.test/live/";
const TS_URL: &str = "http://example/live.ts";
const STALE_DIGEST: &str = "Digest realm=\"r\", nonce=\"n1\", qop=\"auth\", stale=true";
const KEY_URL_OTHER_ORIGIN: &str = "http://keys.test/key.bin";
const ENCRYPTED_SEGMENT: &str = "seg0.ts";
const IV: &str = "0x000102030405060708090A0B0C0D0E0F";
const CHUNK: usize = 1316;
const STEP_US: u64 = 3_200;
const TICK_EVERY: usize = 16;
const TAIL_TICKS: u64 = 120;
const TICK_US: u64 = 50_000;

fn actions(session: &mut Session, now: u64) -> Vec<Action> {
    let mut out = Vec::new();
    while let Some(a) = session.poll(ts(now)) {
        out.push(a);
    }
    out
}

fn close_conns(actions: &[Action]) -> Vec<ConnId> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Close { conn } => Some(*conn),
            _ => None,
        })
        .collect()
}

fn encrypted_playlist(key_uri: &str) -> Vec<u8> {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-KEY:METHOD=AES-128,URI=\"{key_uri}\",IV={IV}\n\
         #EXTINF:2.0,\n{ENCRYPTED_SEGMENT}\n#EXT-X-ENDLIST\n"
    )
    .into_bytes()
}

#[test]
fn hls_close_drops_queued_actions_and_closes_the_live_request_once() {
    let mut session =
        Session::open(Url::parse(TS_PLAYLIST_URL).unwrap(), hls_config(), ts(0)).unwrap();
    let first = session.poll(ts(0)).expect("first action");
    let conn = match first {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    session.close(ts(0));
    let rest = actions(&mut session, 0);
    assert!(
        rest.iter().all(|a| matches!(a, Action::Close { .. })),
        "{rest:?}"
    );
    assert_eq!(close_conns(&rest), vec![conn], "{rest:?}");
    assert!(session.poll(ts(1)).is_none());
    assert!(session.next_event().is_none());
}

#[test]
fn rtsp_close_drops_queued_sends_and_closes_the_connection_once() {
    let mut session =
        Session::open(Url::parse(RTSP_URL).unwrap(), Config::default(), ts(0)).unwrap();
    let conn = match session.poll(ts(0)) {
        Some(Action::TcpConnect { conn, .. }) => conn,
        other => panic!("{other:?}"),
    };
    session.on_connected(conn);
    session.close(ts(0));
    let rest = actions(&mut session, 0);
    assert!(
        rest.iter().all(|a| matches!(a, Action::Close { .. })),
        "a queued request survived close: {rest:?}"
    );
    assert_eq!(close_conns(&rest), vec![conn], "{rest:?}");
}

#[test]
fn hls_stale_rechallenge_is_replayed_at_most_once_then_rejected() {
    let challenge = support::headers(&[("WWW-Authenticate", STALE_DIGEST)]);
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![support::hls_host::status_with(401, challenge)],
    );
    h.session
        .set_credentials(ts(0), Credentials::new("user", "pass"))
        .unwrap();
    h.run_until(|_| false, 400);
    assert!(
        h.hits(TS_PLAYLIST_URL) <= 3,
        "unbounded stale replay: {} requests",
        h.hits(TS_PLAYLIST_URL)
    );
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::AuthRejected { status } if status.as_u16() == 401)),
        "{:?}",
        h.events
    );
}

fn rtsp_cseq(request: &[u8]) -> u32 {
    String::from_utf8_lossy(request)
        .lines()
        .find_map(|l| l.strip_prefix("CSeq:").and_then(|v| v.trim().parse().ok()))
        .unwrap_or(0)
}

#[test]
fn rtsp_stale_rechallenge_is_replayed_at_most_once_then_rejected() {
    let mut session =
        Session::open(Url::parse(RTSP_URL).unwrap(), Config::default(), ts(0)).unwrap();
    session
        .set_credentials(ts(0), Credentials::new("user", "pass"))
        .unwrap();
    let mut describes = 0usize;
    let mut events = Vec::new();
    for _ in 0..40 {
        let batch = actions(&mut session, 0);
        if batch.is_empty() {
            break;
        }
        for action in batch {
            match action {
                Action::TcpConnect { conn, .. } => session.on_connected(conn),
                Action::Send { conn, bytes } => {
                    if bytes.starts_with(b"DESCRIBE") {
                        describes += 1;
                    }
                    let reply = format!(
                        "RTSP/1.0 401 Unauthorized\r\nCSeq: {}\r\nWWW-Authenticate: {STALE_DIGEST}\r\n\r\n",
                        rtsp_cseq(&bytes)
                    );
                    session.on_data(conn, reply.as_bytes(), ts(0));
                }
                _ => {}
            }
        }
        while let Some(e) = session.next_event() {
            events.push(e);
        }
    }
    assert!(
        describes <= 3,
        "unbounded stale replay: {describes} DESCRIBEs"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::AuthRejected { status } if status.as_u16() == 401)),
        "{events:?}"
    );
}

fn renditions_host(config: Config) -> Host {
    let f = fixture();
    let master = "#EXTM3U\n\
        #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"a/index.m3u8\"\n\
        #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Mux\",LANGUAGE=\"de\",DEFAULT=NO,AUTOSELECT=YES\n\
        #EXT-X-STREAM-INF:BANDWIDTH=800000,AUDIO=\"aud\"\nv/index.m3u8\n";
    let mut h = Host::new(MASTER_URL, config);
    h.ok(MASTER_URL, master.as_bytes().to_vec());
    let bodies: HashMap<String, Vec<u8>> = f
        .video
        .bodies(VIDEO_DIR, VIDEO_PLAYLIST_URL)
        .into_iter()
        .chain(f.audio.bodies(AUDIO_DIR, AUDIO_PLAYLIST_URL))
        .collect();
    for (u, b) in bodies {
        h.ok(&u, b);
    }
    h
}

#[test]
fn audio_rendition_switch_after_ended_is_ignored() {
    let mut h = renditions_host(hls_config());
    h.run_to_end();
    assert!(h.events.iter().any(is_ended));
    h.collect();
    let before = h.events.len();
    let requests_before = h.seen.len();
    assert!(h.session.select_audio_rendition(ts(h.now), Some(1)).is_ok());
    assert!(h.session.select_audio_rendition(ts(h.now), None).is_ok());
    h.run_until(|_| false, 50);
    assert_eq!(
        h.events.len(),
        before,
        "events after Ended: {:?}",
        &h.events[before..]
    );
    assert_eq!(h.seen.len(), requests_before, "requests after Ended");
}

#[test]
fn nothing_is_emitted_after_a_fatal_error_in_the_same_poll() {
    let config = {
        let mut new_config = hls_config();
        new_config.max_key_failures = 1;
        new_config
    };
    let master = "#EXTM3U\n\
        #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"a/index.m3u8\"\n\
        #EXT-X-STREAM-INF:BANDWIDTH=800000,AUDIO=\"aud\"\nv/index.m3u8\n";
    let mut h = Host::new(MASTER_URL, config);
    h.ok(MASTER_URL, master.as_bytes().to_vec());
    h.ok(VIDEO_PLAYLIST_URL, encrypted_playlist("key.bin"));
    h.route(&format!("{VIDEO_DIR}/key.bin"), vec![Reply::Hang]);
    h.route(
        &format!("{VIDEO_DIR}/{ENCRYPTED_SEGMENT}"),
        vec![Reply::Hang],
    );
    h.route(AUDIO_PLAYLIST_URL, vec![Reply::Hang]);
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Error { fatal: true, .. }))
        },
        200,
    );
    h.run_until(|_| false, 50);
    let fatal_at = h
        .events
        .iter()
        .position(|e| matches!(e, Event::Error { fatal: true, .. }))
        .unwrap_or_else(|| panic!("no fatal error: {:?}", h.events));
    assert_eq!(
        h.events.len(),
        fatal_at + 1,
        "events emitted after the fatal error: {:?}",
        &h.events[fatal_at + 1..]
    );
}

#[test]
fn cross_origin_key_401_reports_key_fetch_failed_once_then_fatal() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.ok(TS_PLAYLIST_URL, encrypted_playlist(KEY_URL_OTHER_ORIGIN));
    h.route(
        KEY_URL_OTHER_ORIGIN,
        vec![support::hls_host::status_only(401)],
    );
    h.ok(&format!("{ORIGIN}/hls/{ENCRYPTED_SEGMENT}"), vec![0u8; 188]);
    h.run_until(
        |evs| {
            evs.iter()
                .any(|e| matches!(e, Event::Error { fatal: true, .. }))
        },
        4000,
    );
    let errors: Vec<(&ErrorKind, bool)> = h
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Error { kind, fatal } => Some((kind, *fatal)),
            _ => None,
        })
        .collect();
    let expected = ErrorKind::KeyFetchFailed {
        status: Some(support::status(401)),
    };
    assert_eq!(
        errors.iter().filter(|(_, fatal)| !fatal).count(),
        1,
        "{errors:?}"
    );
    assert_eq!(errors.first(), Some(&(&expected, false)), "{errors:?}");
    assert_eq!(errors.last(), Some(&(&expected, true)), "{errors:?}");
}

fn live_events(session: &mut Session, conn: ConnId, bytes: &[u8], base_us: u64) -> Vec<Event> {
    let mut out = Vec::new();
    let mut now = base_us;
    for (i, chunk) in bytes.chunks(CHUNK).enumerate() {
        now = base_us + (i as u64) * STEP_US;
        session.on_data(conn, chunk, ts(now));
        if i % TICK_EVERY == 0 {
            session.on_playback_position(ts(now), None);
        }
        while let Some(e) = session.next_event() {
            out.push(e);
        }
    }
    for k in 1..=TAIL_TICKS {
        session.on_playback_position(ts(now + k * TICK_US), None);
        while let Some(e) = session.next_event() {
            out.push(e);
        }
    }
    out
}

fn first_request(session: &mut Session, now: u64) -> ConnId {
    actions(session, now)
        .into_iter()
        .find_map(|a| match a {
            Action::HttpRequest { conn, .. } => Some(conn),
            _ => None,
        })
        .expect("request")
}

fn buffer_events(events: &[Event]) -> Vec<BufferEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Buffer(b) => Some(b.clone()),
            _ => None,
        })
        .collect()
}

fn second_connection_buffer_events(first_headers: &http::HeaderMap) -> Option<Vec<BufferEvent>> {
    let bytes = support::read_tf1_bytes()?;
    let (status, live_headers) = support::read_tf1_headers()?;
    let mut session = Session::open(Url::parse(TS_URL).unwrap(), Config::default(), ts(0)).unwrap();
    let first = first_request(&mut session, 0);
    session.on_response(first, status, first_headers, ts(0));
    session.on_data(first, &bytes[..bytes.len() / 4], ts(0));
    while session.next_event().is_some() {}
    let later = 10_000_000;
    session.reconnect(ts(later));
    let second = first_request(&mut session, later);
    session.on_response(second, status, &live_headers, ts(0));
    Some(buffer_events(&live_events(
        &mut session,
        second,
        &bytes,
        later,
    )))
}

#[test]
fn live_connection_after_a_finite_one_is_buffered_like_after_a_live_one() {
    let bytes = tf1_bytes!();
    let (_, live_headers) = tf1_headers!();
    let mut finite_headers = live_headers.clone();
    finite_headers.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&bytes.len().to_string()).expect("a test writes a length"),
    );

    let expected = skip_none!(second_connection_buffer_events(&live_headers));
    assert!(
        expected
            .iter()
            .any(|e| matches!(e, BufferEvent::Start { .. })),
        "the live-after-live control never started"
    );
    let got = skip_none!(second_connection_buffer_events(&finite_headers));
    assert_eq!(
        got, expected,
        "the finite connection's gating leaked into the live connection"
    );
}

#[test]
fn queued_events_are_capped_by_dropping_the_oldest_samples() {
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let drive = |config: Config| {
        let mut s = Session::open(Url::parse(TS_URL).unwrap(), config, ts(0)).unwrap();
        let c = first_request(&mut s, 0);
        s.on_response(c, status, &headers, ts(0));
        for chunk in bytes.chunks(CHUNK) {
            s.on_data(c, chunk, ts(0));
        }
        let mut evs = Vec::new();
        while let Some(e) = s.next_event() {
            evs.push(e);
        }
        (evs, s.dropped_events())
    };
    let (all, none_dropped) = drive(Config::default());
    assert_eq!(none_dropped, 0);
    let key = |e: &Event| match e {
        Event::Sample { track, sample, .. } => Some((
            *track,
            sample.timing.and_then(|t| t.pts()),
            sample.timing.and_then(|t| t.dts()),
        )),
        _ => None,
    };
    let all_samples: Vec<(TrackId, Option<i64>, Option<i64>)> =
        all.iter().filter_map(key).collect();
    assert!(all_samples.len() > 200, "fixture too small");

    let cap = 100;
    let capped_cfg = {
        let mut new_config = Config::default();
        new_config.max_queued_events = cap;
        new_config
    };
    let (capped, dropped) = drive(capped_cfg);
    let kept: Vec<_> = capped.iter().filter_map(key).collect();
    assert!(dropped > 0);
    assert_eq!(
        u64::try_from(all_samples.len() - kept.len()).unwrap(),
        dropped
    );
    assert_eq!(kept[..], all_samples[all_samples.len() - kept.len()..]);
    assert!(
        capped.len() - kept.len() >= 3,
        "non-sample events were dropped: {capped:?}"
    );
    assert!(
        capped
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Live)))
    );
    assert!(capped.iter().any(|e| matches!(e, Event::Tracks(_))));
}

#[test]
fn queued_sample_bytes_are_capped_by_dropping_the_oldest_samples() {
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let drive = |config: Config| {
        let mut s = Session::open(Url::parse(TS_URL).unwrap(), config, ts(0)).unwrap();
        let c = first_request(&mut s, 0);
        s.on_response(c, status, &headers, ts(0));
        for chunk in bytes.chunks(CHUNK) {
            s.on_data(c, chunk, ts(0));
        }
        let mut evs = Vec::new();
        while let Some(e) = s.next_event() {
            evs.push(e);
        }
        (evs, s.dropped_events())
    };
    let sample_bytes = |evs: &[Event]| -> usize {
        evs.iter()
            .map(|e| match e {
                Event::Sample { sample, .. } => sample.data.len(),
                _ => 0,
            })
            .sum()
    };
    let (all, none_dropped) = drive(Config::default());
    assert_eq!(none_dropped, 0);
    let total = sample_bytes(&all);
    let budget = total / 4;
    let capped_cfg = {
        let mut new_config = Config::default();
        new_config.max_queued_bytes = budget;
        new_config
    };
    let (capped, dropped) = drive(capped_cfg);
    assert!(dropped > 0, "a quarter of the bytes must shed samples");
    assert!(
        sample_bytes(&capped) <= budget,
        "queued sample bytes {} exceed the budget {budget}",
        sample_bytes(&capped)
    );
    assert!(capped.iter().any(|e| matches!(e, Event::Discontinuity)));
    assert!(capped.iter().any(|e| matches!(e, Event::Tracks(_))));
}

#[test]
fn kind_is_unaffected_by_rtsp_url_with_forced_ts_http() {
    let config = {
        let mut new_config = Config::default();
        new_config.kind = SourceKind::TsHttp;
        new_config
    };
    assert!(matches!(
        Session::open(Url::parse(RTSP_URL).unwrap(), config, ts(0)),
        Err(broadcast_client::error::Error::UnsupportedScheme { .. })
    ));
}

fn rtsp_wake_only_next(host: &RtspHost) -> Option<u64> {
    rtsp_wakes(&host.actions)
        .into_iter()
        .filter(|t| *t > host.now)
        .min()
}

#[test]
fn rtsp_wake_only_host_sees_start_timeout_and_reconnects() {
    let mut host = RtspHost::new(
        BASE_URL,
        Config::default(),
        Server::new(&MediaFixture::load().sdp),
    );
    host.silent = true;
    host.connect();
    let first = rtsp_connects(&host.actions).len();
    assert_eq!(first, 1);
    for _ in 0..8 {
        let Some(next) = rtsp_wake_only_next(&host) else {
            break;
        };
        host.advance_to(next);
        if rtsp_connects(&host.actions).len() > first {
            break;
        }
    }
    assert!(
        rtsp_connects(&host.actions).len() > first,
        "no reconnect for a host that only wakes at WakeAt: {:?}",
        host.actions
    );
    let events = host.events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))),
        "{events:?}"
    );
}

#[test]
fn rtsp_wake_only_host_sees_stall_notice_and_stall_reconnect() {
    let fixture = MediaFixture::load();
    let mut server = Server::new(&fixture.sdp);
    server.session_timeout = Some(100_000);
    let mut host = RtspHost::new(BASE_URL, Config::default(), server);
    host.connect();
    host.advance_to(RTSP_SEC);
    host.deliver_all(&fixture.frames()[..60]);
    host.events();
    let connects_before = rtsp_connects(&host.actions).len();
    let mut stalled = false;
    for _ in 0..80 {
        let Some(next) = rtsp_wake_only_next(&host) else {
            break;
        };
        host.advance_to(next);
        stalled |= host
            .events()
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Stalled { .. })));
        if rtsp_connects(&host.actions).len() > connects_before {
            break;
        }
    }
    assert!(stalled, "no stall notice reached a wake-only host");
    assert!(
        rtsp_connects(&host.actions).len() > connects_before,
        "no stall reconnect for a wake-only host"
    );
}

const TEN_HOURS_US: u64 = 10 * 3_600 * 1_000_000;
const MAX_NOTICES_PER_JUMP: usize = 70;
const FIRST_STALL_NOTICE_US: u64 = 6_000_000;

fn stalled_count(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::Status(Status::Stalled { .. })))
        .count()
}

#[test]
fn ts_time_jump_does_not_flood_stall_notices() {
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut session = Session::open(Url::parse(TS_URL).unwrap(), Config::default(), ts(0)).unwrap();
    let conn = first_request(&mut session, 0);
    session.on_response(conn, status, &headers, ts(0));
    for chunk in bytes.chunks(65_536) {
        session.on_data(conn, chunk, ts(0));
    }
    while session.next_event().is_some() {}
    actions(&mut session, FIRST_STALL_NOTICE_US);
    while session.next_event().is_some() {}
    actions(&mut session, TEN_HOURS_US);
    let mut events = Vec::new();
    while let Some(e) = session.next_event() {
        events.push(e);
    }
    assert!(
        stalled_count(&events) <= MAX_NOTICES_PER_JUMP,
        "{} stall notices for one poll",
        stalled_count(&events)
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))
                || matches!(e, Event::Discontinuity)
                || stalled_count(&events) > 0),
        "{events:?}"
    );
}

#[test]
fn ts_reconnect_past_the_end_of_the_clock_emits_no_wake() {
    let mut session =
        Session::open(Url::parse(TS_URL).unwrap(), Config::default(), ts(0)).expect("open");
    let req = crate::support::ts_probe::one(&mut session, 0);
    session.on_response(
        req.conn,
        support::status(503),
        &support::headers(&[]),
        ts(u64::MAX - 1),
    );
    while session.next_event().is_some() {}
    let at = u64::MAX / 2;
    let wakes: Vec<Timestamp> = actions(&mut session, at)
        .into_iter()
        .filter_map(|a| match a {
            Action::WakeAt(t) => Some(t),
            _ => None,
        })
        .collect();
    assert!(
        wakes
            .iter()
            .all(|w| *w < Timestamp::from_micros(u64::MAX / 2)),
        "the backoff deadline saturates to the last instant, which reads as due now forever: {wakes:?}"
    );
}

#[test]
fn ts_time_jump_to_the_end_of_time_terminates() {
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut session = Session::open(Url::parse(TS_URL).unwrap(), Config::default(), ts(0)).unwrap();
    let conn = first_request(&mut session, 0);
    session.on_response(conn, status, &headers, ts(0));
    for chunk in bytes.chunks(65_536) {
        session.on_data(conn, chunk, ts(0));
    }
    while session.next_event().is_some() {}
    actions(&mut session, FIRST_STALL_NOTICE_US);
    while session.next_event().is_some() {}
    actions(&mut session, u64::MAX);
    let mut count = 0usize;
    while session.next_event().is_some() {
        count += 1;
    }
    assert!(count <= 2 * MAX_NOTICES_PER_JUMP, "{count} events");
}

#[test]
fn rtsp_time_jump_does_not_flood_stall_notices() {
    let fixture = MediaFixture::load();
    let mut server = Server::new(&fixture.sdp);
    server.session_timeout = Some(100_000);
    let mut host = RtspHost::new(BASE_URL, Config::default(), server);
    host.connect();
    host.advance_to(RTSP_SEC);
    host.deliver_all(&fixture.frames()[..30]);
    host.events();
    let last_byte = host.now;
    host.advance_to(last_byte + RTSP_SEC * 6);
    host.events();
    let mut cfg_safe = 0usize;
    host.advance_to(last_byte + TEN_HOURS_US);
    let events = host.events();
    cfg_safe += stalled_count(&events);
    assert!(
        cfg_safe <= MAX_NOTICES_PER_JUMP,
        "{cfg_safe} stall notices for one poll"
    );
}

const RUNAWAY_POLLS: usize = 1_000;

fn polls_until_idle(session: &mut Session, now: Timestamp) -> usize {
    let mut polls = 0usize;
    while session.poll(now).is_some() {
        polls += 1;
        if polls >= RUNAWAY_POLLS {
            break;
        }
    }
    polls
}

#[test]
fn ts_poll_at_the_end_of_time_goes_idle() {
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut session = Session::open(Url::parse(TS_URL).unwrap(), Config::default(), ts(0)).unwrap();
    let conn = first_request(&mut session, 0);
    session.on_response(conn, status, &headers, ts(0));
    for chunk in bytes.chunks(65_536) {
        session.on_data(conn, chunk, ts(0));
    }
    polls_until_idle(&mut session, ts(FIRST_STALL_NOTICE_US));
    while session.next_event().is_some() {}
    let polls = polls_until_idle(&mut session, Timestamp::from_nanos(u64::MAX));
    assert!(polls < RUNAWAY_POLLS, "poll never went idle at u64::MAX");
}

#[test]
fn rtsp_poll_at_the_end_of_time_goes_idle() {
    let fixture = MediaFixture::load();
    let mut server = Server::new(&fixture.sdp);
    server.session_timeout = Some(100_000);
    let mut host = RtspHost::new(BASE_URL, Config::default(), server);
    host.connect();
    host.advance_to(RTSP_SEC);
    host.deliver_all(&fixture.frames()[..30]);
    host.events();
    let mut polls = 0usize;
    while host.session.poll(Timestamp::from_nanos(u64::MAX)).is_some() {
        polls += 1;
        if polls >= RUNAWAY_POLLS {
            break;
        }
    }
    assert!(polls < RUNAWAY_POLLS, "poll never went idle at u64::MAX");
}

#[test]
fn ts_session_started_at_the_end_of_time_goes_idle() {
    let bytes = tf1_bytes!();
    let (status, headers) = tf1_headers!();
    let mut session = Session::open(
        Url::parse(TS_URL).unwrap(),
        Config::default(),
        Timestamp::from_nanos(u64::MAX),
    )
    .unwrap();
    let conn = match session.poll(Timestamp::from_nanos(u64::MAX)) {
        Some(Action::HttpRequest { conn, .. }) => conn,
        other => panic!("{other:?}"),
    };
    session.on_response(conn, status, &headers, Timestamp::from_nanos(u64::MAX));
    for chunk in bytes.chunks(65_536) {
        session.on_data(conn, chunk, Timestamp::from_nanos(u64::MAX));
    }
    assert!(
        std::iter::from_fn(|| session.next_event()).any(|e| matches!(e, Event::Sample { .. })),
        "fixture never reached Playing"
    );
    let polls = polls_until_idle(&mut session, Timestamp::from_nanos(u64::MAX));
    assert!(polls < RUNAWAY_POLLS, "poll never went idle at u64::MAX");
}

#[test]
fn rtsp_session_started_at_the_end_of_time_goes_idle() {
    let fixture = MediaFixture::load();
    let mut server = Server::new(&fixture.sdp);
    server.session_timeout = Some(100_000);
    let mut host = RtspHost::new(BASE_URL, Config::default(), server);
    host.now = u64::MAX;
    host.connect();
    host.deliver_all(&fixture.frames()[..30]);
    host.events();
    let mut polls = 0usize;
    while host.session.poll(Timestamp::from_nanos(u64::MAX)).is_some() {
        polls += 1;
        if polls >= RUNAWAY_POLLS {
            break;
        }
    }
    assert!(polls < RUNAWAY_POLLS, "poll never went idle at u64::MAX");
}

fn assert_wakes_in_the_future(actions: &[Action], now: u64) {
    for a in actions {
        if let Action::WakeAt(t) = a {
            assert!(*t > ts(now), "WakeAt({t:?}) is not after now={now}");
        }
    }
}

#[test]
fn ts_never_emits_a_wake_at_or_before_now() {
    let mut session = Session::open(Url::parse(TS_URL).unwrap(), Config::default(), ts(0)).unwrap();
    let conn = first_request(&mut session, 0);
    session.on_response(
        conn,
        support::status(200),
        &support::headers(&[]),
        Timestamp::from_nanos(u64::MAX),
    );
    for now in [1_000_000u64, 2_000_000, u64::MAX] {
        session.reconnect(ts(now));
        let batch = actions(&mut session, now);
        assert_wakes_in_the_future(&batch, now);
    }
}

#[test]
fn rtsp_never_emits_a_wake_at_or_before_now() {
    let mut session =
        Session::open(Url::parse(RTSP_URL).unwrap(), Config::default(), ts(0)).unwrap();
    let conn = match session.poll(ts(0)) {
        Some(Action::TcpConnect { conn, .. }) => conn,
        other => panic!("{other:?}"),
    };
    session.on_connected(conn);
    for now in [1_000_000u64, 2_000_000, u64::MAX] {
        session.reconnect(ts(now));
        let batch: Vec<Action> = std::iter::from_fn(|| session.poll(ts(now)))
            .take(RUNAWAY_POLLS)
            .collect();
        assert_wakes_in_the_future(&batch, now);
    }
}

#[test]
fn hls_never_emits_a_wake_at_or_before_now() {
    let mut session =
        Session::open(Url::parse(TS_PLAYLIST_URL).unwrap(), hls_config(), ts(0)).unwrap();
    for now in [0u64, 1_000_000, u64::MAX] {
        let batch: Vec<Action> = std::iter::from_fn(|| session.poll(ts(now)))
            .take(RUNAWAY_POLLS)
            .collect();
        assert_wakes_in_the_future(&batch, now);
        session.reconnect(ts(now));
        let batch: Vec<Action> = std::iter::from_fn(|| session.poll(ts(now)))
            .take(RUNAWAY_POLLS)
            .collect();
        assert_wakes_in_the_future(&batch, now);
    }
}
