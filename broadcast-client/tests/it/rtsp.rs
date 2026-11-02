use crate::support;

use std::sync::OnceLock;

use broadcast_auth::{Credentials as AuthCredentials, Scheme as AuthScheme, Verifier};
use broadcast_client::{Action, Config, Credentials, Error, Event, Session, SourceKind, Status};
use rtsp_runtime::transport::{LowerTransport, Transport};
use sdp_types::{Media, MediaType, Origin, TransportProto};
use support::rtsp_host::{
    BASE_URL, Chunking, DESCRIBE, Host, MediaFixture, OPTIONS, PLAY, SEC, SETUP, Server, TEARDOWN,
    channel_of, connects, interleaved, quiet_config, samples_of, sender_report_frame, session_id,
    wakes,
};
use url::Url;

const USER: &str = "admin";
const PASSWORD: &str = "s3cret";
const BASIC_AUTHORIZATION: &str = "Basic YWRtaW46czNjcmV0";
const REALM: &str = "cam";
const STALL_NOTICE: u64 = 5 * SEC;
const STALL_RECONNECT: u64 = 30 * SEC;
const START_TIMEOUT: u64 = 15 * SEC;
const BACKOFF_SECS: [u64; 6] = [2, 4, 8, 10, 10, 10];
const SOME_FRAMES: usize = 60;
const SLOW_SESSION_TIMEOUT: u32 = 100_000;
const MIN_RECONNECT: u64 = SEC;

fn fixture() -> &'static MediaFixture {
    static FIXTURE: OnceLock<MediaFixture> = OnceLock::new();
    FIXTURE.get_or_init(MediaFixture::load)
}

fn server() -> Server {
    Server::new(&fixture().sdp)
}

fn connected(config: Config, server: Server) -> Host {
    let mut host = Host::new(BASE_URL, config, server);
    host.connect();
    host
}

fn playing() -> Host {
    connected(Config::default(), server())
}

fn statuses(events: &[Event]) -> Vec<Status> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Status(s) => Some(*s),
            _ => None,
        })
        .collect()
}

fn reconnecting_attempts(events: &[Event]) -> Vec<u64> {
    statuses(events)
        .into_iter()
        .filter_map(|s| match s {
            Status::Reconnecting { attempt } if attempt > 0 => Some(attempt),
            _ => None,
        })
        .collect()
}

fn closes(host: &Host) -> usize {
    host.count_actions(|a| matches!(a, Action::Close { .. }))
}

fn tcp_connects(host: &Host) -> usize {
    connects(&host.actions).len()
}

fn options_at(host: &mut Host, due_us: u64) -> usize {
    host.advance_to(due_us - 1);
    assert_eq!(
        host.server.count(OPTIONS),
        0,
        "no keepalive before {due_us} us"
    );
    host.advance_to(due_us);
    host.server.count(OPTIONS)
}

fn sdp_with(medias: Vec<Media>) -> String {
    let mut session = sdp_types::Session::new(
        Origin::with_ip_addr(0, 0, std::net::Ipv4Addr::LOCALHOST),
        "t",
    );
    session.medias = medias;
    let mut out = Vec::new();
    session.write(&mut out).expect("write sdp");
    String::from_utf8(out).expect("utf8")
}

fn basic_verifier() -> Verifier {
    Verifier::new(
        AuthCredentials::Basic {
            username: USER.to_string(),
            password: PASSWORD.to_string(),
        },
        REALM,
    )
}

fn digest_verifier() -> Verifier {
    Verifier::new(
        AuthCredentials::Digest {
            username: USER.to_string(),
            password: PASSWORD.to_string(),
        },
        REALM,
    )
}

fn authed_server(verifier: Verifier) -> Server {
    let mut s = server();
    s.auth = Some(verifier);
    s
}

fn user_credentials(password: &str) -> Credentials {
    Credentials::new(USER, password)
}

fn finish_stream(host: &mut Host) -> Vec<Event> {
    finish_stream_after(host, 0)
}

fn finish_stream_after(host: &mut Host, lasted_us: u64) -> Vec<Event> {
    let frames = fixture().frames();
    host.deliver_all(&frames);
    host.advance(lasted_us);
    let conn = host.conn.expect("live connection");
    host.session.on_closed(conn, None, host.now);
    host.events()
}

#[test]
fn auto_detects_rtsp_and_rtsps_schemes() {
    for url in ["rtsp://cam.test/live/", "rtsps://cam.test/live/"] {
        let mut session = Session::open(url, Config::default(), 0).expect("open");
        assert!(
            matches!(session.poll(0), Some(Action::TcpConnect { .. })),
            "{url} must start with a TCP connect under Auto"
        );
        let mut explicit = Session::open(
            url,
            {
                let mut new_config = Config::default();
                new_config.kind = SourceKind::Rtsp;
                new_config
            },
            0,
        )
        .expect("open explicit");
        assert!(matches!(explicit.poll(0), Some(Action::TcpConnect { .. })));
    }
    let mut http = Session::open("http://cam.test/live.ts", Config::default(), 0).expect("open");
    assert!(matches!(http.poll(0), Some(Action::HttpRequest { .. })));
    assert_eq!(SourceKind::Rtsp.to_string(), "rtsp");
    for (url, kind) in [
        ("rtsp://cam.test/x", SourceKind::TsHttp),
        ("rtsp://cam.test/x", SourceKind::Hls),
        ("http://cam.test/x", SourceKind::Rtsp),
    ] {
        let config = {
            let mut new_config = Config::default();
            new_config.kind = kind;
            new_config
        };
        assert!(
            matches!(
                Session::open(url, config, 0),
                Err(Error::UnsupportedScheme(_))
            ),
            "{url} with {kind} must be rejected"
        );
    }
}

#[test]
fn connect_action_has_host_port_default_ports_and_tls_flag() {
    let cases = [
        ("rtsp://cam.test/live/", "cam.test", 554, false),
        ("rtsp://cam.test:8554/live/", "cam.test", 8554, false),
        ("rtsps://cam.test/live/", "cam.test", 322, true),
        ("rtsps://cam.test:1322/live/", "cam.test", 1322, true),
        ("rtsp://user:pw@10.0.0.7/live/", "10.0.0.7", 554, false),
        ("rtsps://[::1]:1322/x", "::1", 1322, true),
    ];
    for (url, host, port, tls) in cases {
        let mut session = Session::open(url, Config::default(), 0).expect("open");
        let first = session.poll(0).expect("connect action");
        assert_eq!(
            first,
            Action::TcpConnect {
                conn: 1,
                host: host.to_string(),
                port,
                tls,
                identity: None,
            },
            "{url}"
        );
    }
    assert!(matches!(
        Session::open("rtsp:///nohost", Config::default(), 0),
        Err(Error::InvalidUrl(_))
    ));
}

#[test]
fn handshake_sequence_describe_setup_per_track_play_in_order() {
    let mut host = Host::new(BASE_URL, Config::default(), server());
    host.drain();
    assert_eq!(
        host.sent_methods(),
        Vec::<String>::new(),
        "nothing is sent before the host reports the connection"
    );
    let conn = host.conn.expect("connect requested");
    host.session.on_connected(conn);
    host.drain();
    assert_eq!(
        host.server.methods(),
        [DESCRIBE, SETUP, SETUP, PLAY],
        "one DESCRIBE, a SETUP per SDP media, then PLAY"
    );
    let log = &host.server.log;
    assert_eq!(log[0].uri, BASE_URL);
    assert_eq!(log[0].header("accept"), Some("application/sdp"));
    assert_eq!(log[1].uri, "rtsp://cam.test/live/trackID=1");
    assert_eq!(log[2].uri, "rtsp://cam.test/live/trackID=2");
    assert_eq!(log[3].uri, BASE_URL);
    assert_eq!(log.iter().map(|r| r.cseq).collect::<Vec<_>>(), [1, 2, 3, 4]);
    assert_eq!(log[1].header("session"), None);
    assert_eq!(log[2].header("session"), Some(session_id()));
    assert_eq!(log[3].header("session"), Some(session_id()));
}

#[test]
fn interleaved_channels_assigned_0_1_2_3() {
    let host = playing();
    let setups: Vec<_> = host
        .server
        .log
        .iter()
        .filter(|r| r.method == SETUP)
        .collect();
    assert_eq!(setups.len(), 2);
    assert_eq!(setups[0].interleaved(), Some((0, 1)));
    assert_eq!(setups[1].interleaved(), Some((2, 3)));
    for setup in setups {
        let transport =
            Transport::parse(setup.header("transport").expect("transport")).expect("parse");
        let spec = transport.first().expect("one spec");
        assert_eq!(spec.lower_transport, Some(LowerTransport::Tcp));
    }
}

#[test]
fn tracks_precede_first_sample_and_samples_equal_oracle() {
    let fx = fixture();
    let mut host = playing();
    let events = finish_stream(&mut host);
    let first_tracks = events
        .iter()
        .position(|e| matches!(e, Event::Tracks(_)))
        .expect("Tracks event");
    let first_sample = events
        .iter()
        .position(|e| matches!(e, Event::Sample { .. }))
        .expect("Sample event");
    assert!(
        first_tracks < first_sample,
        "Tracks must precede the first Sample"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Tracks(_)))
            .count(),
        1,
        "the track set is announced once"
    );
    let Event::Tracks(specs) = &events[first_tracks] else {
        unreachable!("position matched Tracks");
    };
    let ids: Vec<u32> = specs.iter().map(|s| s.track_id.value()).collect();
    assert_eq!(ids, [1, 2]);
    let got = samples_of(&events);
    for (index, track) in fx.tracks.iter().enumerate() {
        let id = u32::try_from(index).expect("small") + 1;
        let mine: Vec<_> = got
            .iter()
            .filter(|(t, _)| *t == id)
            .map(|(_, s)| s)
            .collect();
        assert_eq!(
            mine.len(),
            track.samples.len(),
            "track {id}: every original sample is recovered"
        );
        let origin = track.samples[0].pts.expect("fixture pts");
        for (n, (recovered, original)) in mine.iter().zip(&track.samples).enumerate() {
            assert_eq!(
                &recovered.data[..],
                &original.data[..],
                "track {id} sample {n}: payload bytes"
            );
            assert_eq!(
                recovered.pts,
                original.pts.map(|p| p - origin),
                "track {id} sample {n}: pts"
            );
            assert_eq!(
                recovered.flags.is_sync, original.flags.is_sync,
                "track {id} sample {n}: sync flag"
            );
        }
    }
}

#[test]
fn chunked_tcp_delivery_1_byte_and_arbitrary_splits_identical_events() {
    let run = |chunking: Chunking| {
        let mut host = Host::new(BASE_URL, Config::default(), server());
        host.chunking = chunking;
        host.connect();
        let events = finish_stream(&mut host);
        assert!(!samples_of(&events).is_empty());
        events
    };
    let whole = run(Chunking::Whole);
    let one_byte = run(Chunking::Cycle(vec![1]));
    let uneven = run(Chunking::Cycle(vec![
        1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377,
    ]));
    assert_eq!(whole.len(), one_byte.len());
    assert!(
        whole == one_byte,
        "1-byte delivery changed the event sequence"
    );
    assert!(whole == uneven, "uneven splits changed the event sequence");
}

fn media_only(events: &[Event]) -> Vec<Event> {
    events
        .iter()
        .filter(|e| !matches!(e, Event::AvSync(_) | Event::Discontinuity))
        .cloned()
        .map(|mut e| {
            if let Event::Sample { sample, join, .. } = &mut e {
                sample.pts = None;
                sample.dts = None;
                *join = None;
            }
            e
        })
        .collect()
}

#[test]
fn rtcp_sender_report_forwarded_without_error() {
    let baseline = {
        let mut host = playing();
        let e = finish_stream(&mut host);
        assert_eq!(
            host.session.rtp_loss_count(),
            0,
            "an intact stream reports no loss"
        );
        e
    };
    let mut host = playing();
    let mut frames = Vec::new();
    for (n, frame) in fixture().frames().into_iter().enumerate() {
        if n % 10 == 0 {
            frames.push(sender_report_frame(channel_of(0) + 1, 0x1234_5678, 90_000));
            frames.push(sender_report_frame(channel_of(1) + 1, 0x8765_4321, 44_100));
            frames.push(interleaved(channel_of(1) + 1, &[0, 0, 0, 0]));
        }
        frames.push(frame);
    }
    host.deliver_all(&frames);
    let conn = host.conn.expect("live");
    host.session.on_closed(conn, None, host.now);
    let events = host.events();
    assert!(
        media_only(&events) == media_only(&baseline),
        "RTCP traffic must not change the media payloads or their order"
    );
    let sent_rtcp = (fixture().frames().len().div_ceil(10) * 3) as u64;
    assert_eq!(
        host.session.rtcp_packet_count(),
        sent_rtcp,
        "every RTCP frame on the RTCP channels is routed to the depayloader"
    );
    assert_eq!(host.session.rtp_loss_count(), 0);
    assert_eq!(
        host.session.rtp_malformed_count(),
        0,
        "RTCP must never reach the RTP depayloader"
    );
    assert!(reconnecting_attempts(&events).is_empty());
}

#[test]
fn status_establishing_then_live_once() {
    let mut host = playing();
    let events = finish_stream(&mut host);
    assert!(samples_of(&events).len() > 2);
    assert_eq!(
        statuses(&events),
        [
            Status::Establishing,
            Status::Live,
            Status::Reconnecting { attempt: 0 }
        ],
        "the finished stream ends a connection that played"
    );
}

#[test]
fn keepalive_options_sent_at_half_session_timeout_and_reply_405_ignored() {
    let mut s = server();
    s.session_timeout = Some(40);
    s.options_status = 405;
    let mut host = connected(quiet_config(), s);
    assert_eq!(options_at(&mut host, 20 * SEC), 1);
    let keepalive = host
        .server
        .log
        .iter()
        .find(|r| r.method == OPTIONS)
        .expect("options");
    assert_eq!(keepalive.uri, BASE_URL);
    assert_eq!(keepalive.header("session"), Some(session_id()));
    host.events();
    let before = closes(&host);
    let frames = fixture().frames();
    host.deliver_all(&frames);
    let events = host.events();
    assert!(
        !samples_of(&events).is_empty(),
        "stream continues after a 405 keepalive reply"
    );
    assert!(reconnecting_attempts(&events).is_empty());
    assert_eq!(
        closes(&host),
        before,
        "a 405 must not tear the connection down"
    );
    host.advance_to(40 * SEC - 1);
    assert_eq!(host.server.count(OPTIONS), 1);
    host.advance_to(40 * SEC);
    assert_eq!(host.server.count(OPTIONS), 2, "the keepalive is re-armed");
}

#[test]
fn keepalive_default_30s_when_no_timeout() {
    let mut host = connected(quiet_config(), server());
    assert_eq!(options_at(&mut host, 30 * SEC), 1);
}

#[test]
fn close_sends_teardown_then_close_conn_once() {
    let mut host = playing();
    host.clear_actions();
    let conn = host.conn.expect("live");
    host.session.close();
    host.drain();
    let order: Vec<&Action> = host.actions.iter().collect();
    assert_eq!(
        order.len(),
        2,
        "TEARDOWN then Close, nothing else: {order:?}"
    );
    assert!(matches!(order[0], Action::Send { conn: c, .. } if *c == conn));
    assert_eq!(order[1], &Action::Close { conn });
    let teardown = host.server.log.last().expect("teardown reached the server");
    assert_eq!(teardown.method, TEARDOWN);
    assert_eq!(teardown.header("session"), Some(session_id()));
    host.session.close();
    assert!(host.session.poll(host.now + SEC).is_none());
    assert!(host.session.next_event().is_none());
}

#[test]
fn auth_basic_challenge_flow() {
    let mut host = connected(Config::default(), authed_server(basic_verifier()));
    let events = host.events();
    let origin = Url::parse(BASE_URL).expect("url");
    assert!(events.iter().any(|e| matches!(
        e,
        Event::AuthRequired { scheme: AuthScheme::Basic, realm: Some(r), origin: o, resource_metadata: None }
            if r == REALM && *o == origin
    )));
    assert!(statuses(&events).contains(&Status::WaitingForCredentials));
    host.clear_actions();
    for minute_second in 1..=600 {
        host.advance_to(minute_second * SEC);
    }
    assert!(
        host.actions.is_empty(),
        "nothing is scheduled while waiting: {:?}",
        host.actions
    );
    assert_eq!(
        host.server.count(DESCRIBE),
        1,
        "a 401 without credentials is not retried"
    );
    host.session
        .set_credentials(user_credentials(PASSWORD))
        .unwrap();
    host.drain();
    let describes: Vec<_> = host
        .server
        .log
        .iter()
        .filter(|r| r.method == DESCRIBE)
        .collect();
    assert_eq!(describes.len(), 2);
    assert_eq!(
        describes[1].header("authorization"),
        Some(BASIC_AUTHORIZATION)
    );
    assert_eq!(describes[1].authorized, Some(true));
    let events = finish_stream(&mut host);
    assert!(
        host.server
            .log
            .iter()
            .skip(1)
            .all(|r| r.authorized == Some(true))
    );
    assert_eq!(host.server.count(PLAY), 1);
    assert!(
        !samples_of(&events).is_empty(),
        "the stream proceeds after authentication"
    );
}

#[test]
fn auth_digest_challenge_flow() {
    let mut host = connected(Config::default(), authed_server(digest_verifier()));
    let events = host.events();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::AuthRequired { scheme: AuthScheme::Digest, realm: Some(r), .. } if r == REALM
    )));
    host.session
        .set_credentials(user_credentials(PASSWORD))
        .unwrap();
    host.drain();
    let second = host
        .server
        .log
        .iter()
        .filter(|r| r.method == DESCRIBE)
        .nth(1)
        .expect("DESCRIBE re-sent");
    let header = second
        .header("authorization")
        .expect("Authorization on the re-sent DESCRIBE");
    assert!(header.starts_with("Digest "), "{header}");
    for needle in [
        "username=\"admin\"",
        "realm=\"cam\"",
        "nonce=\"",
        "response=\"",
        "uri=\"rtsp://cam.test/live/\"",
    ] {
        assert!(header.contains(needle), "{needle} missing in {header}");
    }
    assert_eq!(
        second.authorized,
        Some(true),
        "the server's own verifier accepts the response"
    );
    let events = finish_stream(&mut host);
    assert!(
        host.server
            .log
            .iter()
            .skip(1)
            .all(|r| r.authorized == Some(true)),
        "every later request verifies too: {:?}",
        host.server
            .log
            .iter()
            .map(|r| (&r.method, r.authorized))
            .collect::<Vec<_>>()
    );
    assert!(!samples_of(&events).is_empty());
}

#[test]
fn credentials_rejected_emits_auth_rejected() {
    let mut host = connected(Config::default(), authed_server(basic_verifier()));
    host.events();
    host.session
        .set_credentials(user_credentials("wrong"))
        .unwrap();
    host.drain();
    let events = host.events();
    assert!(events.contains(&Event::AuthRejected { status: 401 }));
    assert!(statuses(&events).contains(&Status::WaitingForCredentials));
    host.clear_actions();
    let sent = host.server.log.len();
    for second in 1..=600 {
        host.advance_to(second * SEC);
    }
    assert!(
        host.actions.is_empty(),
        "rejected credentials wait for new ones: {:?}",
        host.actions
    );
    assert_eq!(host.server.log.len(), sent);
    host.session
        .set_credentials(user_credentials(PASSWORD))
        .unwrap();
    host.drain();
    assert_eq!(
        host.server.count(PLAY),
        1,
        "new credentials resume the handshake"
    );
}

#[test]
fn forbidden_403_event() {
    let mut s = server();
    s.describe_status = Some(403);
    let mut host = connected(Config::default(), s);
    let events = host.events();
    assert!(events.contains(&Event::Forbidden { status: 403 }));
    assert_eq!(reconnecting_attempts(&events), [1]);
    assert_eq!(closes(&host), 1);
    assert_eq!(wakes(&host.actions).last(), Some(&(2 * SEC)));
}

#[test]
fn describe_404_status_not_found_and_backoff() {
    let mut s = server();
    s.describe_status = Some(404);
    let mut host = connected(Config::default(), s);
    let events = host.events();
    assert_eq!(
        statuses(&events),
        [
            Status::Establishing,
            Status::NotFound,
            Status::Reconnecting { attempt: 1 }
        ]
    );
    assert_eq!(wakes(&host.actions).last(), Some(&(2 * SEC)));
    host.advance_to(2 * SEC - 1);
    assert_eq!(tcp_connects(&host), 1);
    host.advance_to(2 * SEC);
    assert_eq!(tcp_connects(&host), 2);
    host.connect();
    let events = host.events();
    assert!(statuses(&events).contains(&Status::NotFound));
    assert_eq!(reconnecting_attempts(&events), [2]);
    assert_eq!(wakes(&host.actions).last(), Some(&(6 * SEC)));
}

#[test]
fn connect_failure_backoff_2_4_8_then_10_forever() {
    let mut host = Host::new(BASE_URL, Config::default(), server());
    host.drain();
    for (index, backoff) in BACKOFF_SECS.iter().enumerate() {
        let conn = host.conn.expect("connect requested");
        let failed_at = host.now;
        host.clear_actions();
        host.session
            .on_closed(conn, Some("refused".to_string()), failed_at);
        host.drain();
        let events = host.events();
        assert_eq!(
            reconnecting_attempts(&events),
            [u64::try_from(index).expect("small") + 1]
        );
        let due = failed_at + backoff * SEC;
        assert_eq!(
            wakes(&host.actions).last(),
            Some(&due),
            "attempt {}",
            index + 1
        );
        host.advance_to(due - 1);
        assert_eq!(
            tcp_connects(&host),
            0,
            "no reconnect before the backoff elapsed"
        );
        host.advance_to(due);
        assert_eq!(tcp_connects(&host), 1, "reconnect exactly at the backoff");
    }
}

#[test]
fn start_timeout_15s_reconnects() {
    let mut host = playing();
    host.events();
    host.advance_to(START_TIMEOUT - 1);
    assert_eq!(closes(&host), 0);
    host.clear_actions();
    host.advance_to(START_TIMEOUT);
    assert_eq!(
        host.sent_methods(),
        [TEARDOWN],
        "an established session is torn down"
    );
    assert_eq!(closes(&host), 1);
    let events = host.events();
    assert_eq!(reconnecting_attempts(&events), [1]);
    host.advance_to(START_TIMEOUT + 2 * SEC);
    assert_eq!(
        tcp_connects(&host),
        1,
        "the reconnect fires after the 2 s backoff"
    );
    host.connect();
    host.events();
    host.clear_actions();
    let second_connect = START_TIMEOUT + 2 * SEC;
    host.advance_to(second_connect + START_TIMEOUT - 1);
    assert_eq!(
        closes(&host),
        0,
        "the start timeout restarts on every connection"
    );
    host.advance_to(second_connect + START_TIMEOUT);
    assert_eq!(closes(&host), 1);
    assert_eq!(reconnecting_attempts(&host.events()), [2]);
}

#[test]
fn stall_5s_notice_and_30s_reconnect_with_teardown_abort() {
    let mut s = server();
    s.session_timeout = Some(SLOW_SESSION_TIMEOUT);
    let mut host = connected(Config::default(), s);
    host.advance_to(SEC);
    let frames = fixture().frames();
    host.deliver_all(&frames[..SOME_FRAMES]);
    let events = host.events();
    assert!(!samples_of(&events).is_empty());
    let last_byte = host.now;
    host.advance_to(last_byte + STALL_NOTICE - 1);
    assert!(
        statuses(&host.events())
            .iter()
            .all(|s| !matches!(s, Status::Stalled { .. }))
    );
    host.advance_to(last_byte + STALL_NOTICE);
    let events = host.events();
    assert!(statuses(&events).contains(&Status::Stalled { secs: 5 }));
    assert!(events.contains(&Event::Starved(broadcast_client::StarveKind::NoData)));
    host.advance_to(last_byte + 6 * SEC);
    assert!(statuses(&host.events()).contains(&Status::Stalled { secs: 6 }));
    host.now = last_byte + 10 * SEC;
    host.deliver(&frames[SOME_FRAMES]);
    assert!(
        statuses(&host.events()).contains(&Status::Live),
        "the next byte ends the stall"
    );
    let resumed = host.now;
    host.clear_actions();
    host.advance_to(resumed + STALL_RECONNECT - 1);
    assert_eq!(closes(&host), 0);
    host.advance_to(resumed + STALL_RECONNECT);
    let methods = host.sent_methods();
    assert_eq!(methods, [TEARDOWN]);
    assert_eq!(closes(&host), 1);
    assert_eq!(tcp_connects(&host), 1, "a played stream reconnects at once");
    let attempts = reconnecting_attempts(&host.events());
    assert!(attempts.is_empty(), "no failure was counted: {attempts:?}");
}

#[test]
fn server_closes_after_play_reconnects_at_once_with_discontinuity() {
    let mut host = playing();
    let first = finish_stream_after(&mut host, MIN_RECONNECT);
    assert!(!samples_of(&first).is_empty());
    assert_eq!(tcp_connects(&host), 1);
    host.drain();
    assert_eq!(tcp_connects(&host), 2, "reconnect without waiting");
    host.connect();
    let frames = fixture().frames();
    host.deliver_all(&frames[..SOME_FRAMES]);
    let events = host.events();
    let discontinuity = events
        .iter()
        .position(|e| *e == Event::Discontinuity)
        .expect("Discontinuity");
    let tracks = events
        .iter()
        .position(|e| matches!(e, Event::Tracks(_)))
        .expect("Tracks again");
    let sample = events
        .iter()
        .position(|e| matches!(e, Event::Sample { .. }))
        .expect("Sample");
    assert!(discontinuity < tracks && tracks < sample);
    assert_eq!(statuses(&events), [Status::Establishing, Status::Live]);
    assert_eq!(host.server.count(PLAY), 2);
}

#[test]
fn reconnect_resets_attempt_counter_after_playing() {
    let mut host = Host::new(BASE_URL, Config::default(), server());
    host.drain();
    for backoff in [2, 4] {
        let conn = host.conn.expect("connect");
        host.session
            .on_closed(conn, Some("refused".to_string()), host.now);
        host.advance_to(host.now + backoff * SEC);
    }
    host.connect();
    let frames = fixture().frames();
    host.deliver_all(&frames[..SOME_FRAMES]);
    assert!(!samples_of(&host.events()).is_empty());
    host.advance(MIN_RECONNECT);
    let conn = host.conn.expect("live");
    host.session.on_closed(conn, None, host.now);
    host.drain();
    host.clear_actions();
    let failed_at = host.now;
    let conn = host.conn.expect("reconnect requested at once");
    host.session
        .on_closed(conn, Some("reset".to_string()), failed_at);
    host.drain();
    assert_eq!(reconnecting_attempts(&host.events()).last(), Some(&1));
    assert_eq!(wakes(&host.actions).last(), Some(&(failed_at + 2 * SEC)));
}

#[test]
fn session_timeout_header_parsed_from_setup() {
    for (declared, keepalive_secs) in [(120u32, 60u64), (4, 2), (2, 1), (20_000, 3_600)] {
        let mut s = server();
        s.session_timeout = Some(declared);
        let mut host = connected(quiet_config(), s);
        assert_eq!(
            options_at(&mut host, keepalive_secs * SEC),
            1,
            "Session timeout={declared} gives a {keepalive_secs} s keepalive"
        );
    }
}

#[test]
fn keepalive_is_strictly_before_a_short_session_timeout() {
    for declared in [1u32, 2, 3, 4, 9] {
        let mut s = server();
        s.session_timeout = Some(declared);
        let mut host = connected(quiet_config(), s);
        let expiry = u64::from(declared) * SEC;
        host.advance_to(expiry - 1);
        assert!(
            host.server.count(OPTIONS) >= 1,
            "Session timeout={declared}: no keepalive before expiry"
        );
    }
}

#[test]
fn unsupported_transport_reply_461_is_failure_with_backoff() {
    let mut s = server();
    s.setup_status = Some(461);
    let mut host = connected(Config::default(), s);
    let events = host.events();
    assert_eq!(reconnecting_attempts(&events), [1]);
    assert_eq!(closes(&host), 1);
    assert_eq!(wakes(&host.actions).last(), Some(&(2 * SEC)));
    assert_eq!(host.server.count(PLAY), 0);
    assert!(samples_of(&events).is_empty());
}

#[test]
fn setup_reply_without_interleaved_tcp_is_failure() {
    let mut s = server();
    s.udp_transport = true;
    let mut host = connected(Config::default(), s);
    assert_eq!(reconnecting_attempts(&host.events()), [1]);
    assert_eq!(
        host.server.count(PLAY),
        0,
        "UDP RTP is not supported, PLAY is never sent"
    );
}

#[test]
fn sdp_without_supported_media_is_failure_not_panic() {
    let application = sdp_with(vec![Media::new(
        MediaType::Application,
        0,
        TransportProto::RtpAvp,
        96,
    )]);
    let video_without_fmtp = sdp_with(vec![Media::new(
        MediaType::Video,
        0,
        TransportProto::RtpAvp,
        96,
    )]);
    for sdp in [
        sdp_with(Vec::new()),
        application,
        video_without_fmtp,
        "this is not sdp".to_string(),
        String::new(),
    ] {
        let mut s = Server::new(&sdp);
        s.sdp = sdp.as_bytes().to_vec();
        let mut host = connected(Config::default(), s);
        assert_eq!(
            reconnecting_attempts(&host.events()),
            [1],
            "unusable SDP {sdp:?} is an ordinary failure"
        );
        assert_eq!(host.server.count(SETUP), 0);
        assert_eq!(closes(&host), 1);
    }
}

#[test]
fn rtp_packet_loss_does_not_stop_stream() {
    let fx = fixture();
    let mut host = playing();
    let mut frames = fx.frames();
    let video_channel = channel_of(0);
    let dropped: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, f)| f[1] == video_channel)
        .map(|(n, _)| n)
        .skip(20)
        .take(3)
        .collect();
    assert_eq!(dropped.len(), 3);
    for n in dropped.iter().rev() {
        frames.remove(*n);
    }
    host.deliver_all(&frames);
    let conn = host.conn.expect("live");
    host.session.on_closed(conn, None, host.now);
    let events = host.events();
    assert!(
        host.session.rtp_loss_count() > 0,
        "the sequence gap is counted"
    );
    assert!(reconnecting_attempts(&events).is_empty());
    assert_eq!(closes(&host), 0);
    let video: Vec<_> = samples_of(&events)
        .into_iter()
        .filter(|(t, _)| *t == 1)
        .map(|(_, s)| s)
        .collect();
    let original = &fx.tracks[0].samples;
    assert!(
        video.len() < original.len(),
        "the damaged access units are dropped"
    );
    assert!(
        video.len() + 5 >= original.len(),
        "only the damaged access units are lost"
    );
    let tail = 10;
    for (got, want) in video.iter().rev().take(tail).zip(original.iter().rev()) {
        assert_eq!(
            &got.data[..],
            &want.data[..],
            "samples after the gap are intact"
        );
    }
    let audio = samples_of(&events)
        .into_iter()
        .filter(|(t, _)| *t == 2)
        .count();
    assert_eq!(
        audio,
        fx.tracks[1].samples.len(),
        "the untouched track loses nothing"
    );
}

#[test]
fn late_callbacks_after_close_ignored() {
    let mut host = playing();
    let conn = host.conn.expect("live");
    host.session.close();
    host.drain();
    host.events();
    let seen = host.actions.len();
    host.session.on_connected(conn);
    for frame in fixture().frames().iter().take(SOME_FRAMES) {
        host.session.on_data(conn, frame, host.now);
    }
    host.session
        .on_closed(conn, Some("late".to_string()), host.now);
    host.session
        .set_credentials(user_credentials(PASSWORD))
        .unwrap();
    host.session.reconnect(host.now);
    assert!(host.session.poll(host.now + 100 * SEC).is_none());
    assert!(host.session.next_event().is_none());
    assert_eq!(host.actions.len(), seen);
}

#[test]
fn unknown_connection_callbacks_ignored() {
    let mut host = playing();
    host.events();
    let unknown = host.conn.expect("live") + 100;
    host.session.on_connected(unknown);
    host.session
        .on_data(unknown, &fixture().frames()[0], host.now);
    host.session
        .on_closed(unknown, Some("x".to_string()), host.now);
    host.drain();
    assert!(host.events().is_empty());
    assert_eq!(closes(&host), 0);
}

#[test]
fn rtsps_requests_tls_flag_and_sends_plaintext() {
    let mut host = Host::new("rtsps://cam.test/live/", Config::default(), server());
    host.drain();
    assert!(matches!(
        host.actions[0],
        Action::TcpConnect {
            tls: true,
            port: 322,
            ..
        }
    ));
    let conn = host.conn.expect("connect");
    host.session.on_connected(conn);
    host.drain();
    let first = host
        .actions
        .iter()
        .find_map(|a| match a {
            Action::Send { bytes, .. } => Some(bytes.clone()),
            _ => None,
        })
        .expect("a request was sent");
    assert!(
        first.starts_with(b"DESCRIBE rtsps://cam.test/live/ RTSP/1.0\r\n"),
        "{}",
        String::from_utf8_lossy(&first)
    );
    let events = finish_stream(&mut host);
    assert!(!samples_of(&events).is_empty());
    assert_eq!(host.server.count(PLAY), 1);
}

#[test]
fn garbage_bytes_before_rtsp_response_are_failure_not_panic() {
    let mut host = Host::new(BASE_URL, Config::default(), server());
    host.silent = true;
    host.connect();
    host.deliver(b"\x01\x02 definitely not RTSP\r\n\r\n");
    host.drain();
    let events = host.events();
    assert_eq!(reconnecting_attempts(&events), [1]);
    assert_eq!(closes(&host), 1);
    assert_eq!(wakes(&host.actions).last(), Some(&(2 * SEC)));
    assert!(samples_of(&events).is_empty());
}

#[test]
fn garbage_after_play_reconnects_at_once() {
    let mut host = playing();
    let frames = fixture().frames();
    host.deliver_all(&frames[..SOME_FRAMES]);
    host.events();
    host.advance(MIN_RECONNECT);
    host.clear_actions();
    host.deliver(b"RTSP/9 nonsense\r\n\r\n");
    host.drain();
    assert_eq!(closes(&host), 1);
    assert_eq!(
        tcp_connects(&host),
        1,
        "a stream that played reconnects at once"
    );
    assert!(reconnecting_attempts(&host.events()).is_empty());
}
