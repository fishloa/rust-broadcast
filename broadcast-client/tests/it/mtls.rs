use std::sync::Arc;

use crate::support::replay::{SampleRec, replay_paced};
use crate::support::tls_pki::{Ca, TlsServer, https_get};
use crate::support::udp_host::TS_PACKET;
use broadcast_client::{
    Action, ClientIdentity, ClientIdentityScope, Config, Error, Event, Session, Status,
};

const SMALL: &str = "ts/h264_aac.ts";
const FAST_BPS: u64 = 100_000_000;
const STEP_US: u64 = 1_000;
const CHUNK: usize = 7 * TS_PACKET;
const HANDLE: u64 = 0xC0FFEE;
const PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nhttps://cdn.example/s0.ts\n";

fn cfg(f: impl FnOnce(&mut Config)) -> Config {
    let mut c = Config::default();
    f(&mut c);
    c
}

fn with_handle(f: impl FnOnce(&mut Config)) -> Config {
    cfg(|c| {
        c.client_identity = Some(ClientIdentity::handle(HANDLE));
        f(c);
    })
}

fn first_http(session: &mut Session, now: u64) -> (u32, String, Option<Arc<ClientIdentity>>) {
    while let Some(action) = session.poll(now) {
        if let Action::HttpRequest {
            conn,
            url,
            identity,
            ..
        } = action
        {
            return (conn, url, identity);
        }
    }
    panic!("an HttpRequest");
}

fn identity_handle(i: &Option<Arc<ClientIdentity>>) -> Option<u64> {
    i.as_ref().and_then(|i| i.handle_id())
}

#[test]
fn every_https_source_offers_the_identity_on_its_first_request() {
    for url in [
        "https://h.example/live.ts",
        "https://h.example/live.m3u8",
        "https://h.example/live.mpd",
    ] {
        let mut s = Session::open(url, with_handle(|_| {}), 0).unwrap();
        let (_, request_url, identity) = first_http(&mut s, 0);
        assert_eq!(request_url, url);
        assert_eq!(identity_handle(&identity), Some(HANDLE), "{url}");
    }
}

#[test]
fn rtsps_offers_the_identity_and_plain_rtsp_does_not() {
    for (url, tls) in [
        ("rtsps://cam.example/live", true),
        ("rtsp://cam.example/live", false),
        ("rtsp://cam.example:322/live", false),
    ] {
        let mut s = Session::open(url, with_handle(|_| {}), 0).unwrap();
        let mut seen = None;
        while let Some(a) = s.poll(0) {
            if let Action::TcpConnect {
                tls: t, identity, ..
            } = a
            {
                assert_eq!(t, tls);
                seen = Some(identity_handle(&identity));
            }
        }
        assert_eq!(seen, Some(tls.then_some(HANDLE)), "{url}");
    }
}

#[test]
fn a_plain_http_request_never_carries_the_identity() {
    let mut s = Session::open("http://h.example/live.ts", with_handle(|_| {}), 0).unwrap();
    let (_, _, identity) = first_http(&mut s, 0);
    assert!(identity.is_none());
}

#[test]
fn without_an_identity_no_action_carries_one() {
    let mut s = Session::open("https://h.example/live.ts", Config::default(), 0).unwrap();
    let (_, _, identity) = first_http(&mut s, 0);
    assert!(identity.is_none());
}

fn redirect_target_identity(scope: ClientIdentityScope, to: &str) -> Option<u64> {
    let mut s = Session::open(
        "https://a.example/live.ts",
        with_handle(|c| c.client_identity_scope = scope),
        0,
    )
    .unwrap();
    let (conn, _, first) = first_http(&mut s, 0);
    assert_eq!(identity_handle(&first), Some(HANDLE));
    s.on_response(conn, 302, &[("Location".to_string(), to.to_string())], 1);
    let (_, url, identity) = first_http(&mut s, 1);
    assert_eq!(url, to);
    identity_handle(&identity)
}

#[test]
fn a_redirect_to_another_origin_gets_the_identity_only_under_any_https() {
    let foreign = "https://b.example/live.ts";
    assert_eq!(
        redirect_target_identity(ClientIdentityScope::SessionOrigin, foreign),
        None
    );
    assert_eq!(
        redirect_target_identity(ClientIdentityScope::AnyHttps, foreign),
        Some(HANDLE)
    );
    assert_eq!(
        redirect_target_identity(
            ClientIdentityScope::SessionOrigin,
            "https://a.example/other.ts"
        ),
        Some(HANDLE),
        "the same origin keeps it"
    );
}

#[test]
fn an_hls_segment_on_a_cdn_origin_gets_the_identity_only_under_any_https() {
    for (scope, want) in [
        (ClientIdentityScope::SessionOrigin, None),
        (ClientIdentityScope::AnyHttps, Some(HANDLE)),
    ] {
        let mut s = Session::open(
            "https://a.example/live.m3u8",
            with_handle(|c| c.client_identity_scope = scope),
            0,
        )
        .unwrap();
        let (conn, _, manifest) = first_http(&mut s, 0);
        assert_eq!(identity_handle(&manifest), Some(HANDLE));
        s.on_response(
            conn,
            200,
            &[(
                "Content-Type".to_string(),
                "application/vnd.apple.mpegurl".to_string(),
            )],
            1,
        );
        s.on_data(conn, PLAYLIST.as_bytes(), 2);
        s.on_closed(conn, None, 3);
        let (_, url, segment) = first_http(&mut s, 3);
        assert_eq!(url, "https://cdn.example/s0.ts");
        assert_eq!(identity_handle(&segment), want, "{scope}");
    }
}

#[test]
fn malformed_identities_fail_open_with_a_typed_error() {
    let bad: [ClientIdentity; 5] = [
        ClientIdentity::pem("not pem", "not pem"),
        ClientIdentity::pem("", ""),
        ClientIdentity::der(Vec::new(), vec![0x30, 0x03, 0x02, 0x01, 0x00]),
        ClientIdentity::der(vec![vec![0x30, 0x03, 0x02, 0x01, 0x00]], Vec::new()),
        ClientIdentity::der(vec![vec![0x00]], vec![0x30, 0x03, 0x02, 0x01, 0x00]),
    ];
    let ca = Ca::new("validity ca");
    let leaf = ca.issue("validity client", true);
    let mixed = [
        ClientIdentity::pem(leaf.chain_pem.clone(), "not a key"),
        ClientIdentity::pem(leaf.chain_pem.clone(), leaf.chain_pem.clone()),
        ClientIdentity::pem("not a certificate", leaf.key_pem.clone()),
        ClientIdentity::pem(leaf.key_pem.clone(), leaf.key_pem.clone()),
        ClientIdentity::der(leaf.chain_der.clone(), vec![0x30, 0x01]),
    ];
    for identity in mixed {
        let config = cfg(|c| c.client_identity = Some(identity));
        assert!(matches!(
            Session::open("https://h.example/live.ts", config, 0).unwrap_err(),
            Error::InvalidConfig {
                field: "client_identity"
            }
        ));
    }
    for identity in [leaf.pem_identity(), leaf.der_identity()] {
        let config = cfg(|c| c.client_identity = Some(identity));
        Session::open("https://h.example/live.ts", config, 0).unwrap();
    }
    for identity in bad {
        let config = cfg(|c| c.client_identity = Some(identity));
        match Session::open("https://h.example/live.ts", config, 0).unwrap_err() {
            Error::InvalidConfig { field } => assert_eq!(field, "client_identity"),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn key_material_never_appears_in_debug_output() {
    let ca = Ca::new("debug ca");
    let leaf = ca.issue("debug client", true);
    let key_body: String = leaf
        .key_pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    assert!(key_body.len() > 40);
    let config = cfg(|c| c.client_identity = Some(leaf.pem_identity()));
    let mut s = Session::open("https://h.example/live.ts", config.clone(), 0).unwrap();
    let (_, _, identity) = first_http(&mut s, 0);
    let action = Action::HttpRequest {
        conn: 1,
        method: "GET".to_string(),
        url: "https://h.example/live.ts".to_string(),
        headers: Vec::new(),
        byte_range: None,
        identity,
    };
    let text = format!("{config:?} {action:?} {s:?}");
    assert!(!text.contains(&key_body[..40]), "{text}");
    assert!(text.contains("redacted"));
    let der_text = format!("{:?}", leaf.der_identity());
    assert!(der_text.contains("redacted"));
}

fn baseline(ts: &[u8]) -> Vec<SampleRec> {
    replay_paced(ts, Config::default(), CHUNK, FAST_BPS).samples()
}

struct Outcome {
    events: Vec<Event>,
    requests: usize,
    offered: usize,
}

fn drive(
    server: &TlsServer,
    server_ca: &Ca,
    config: Config,
    attempts: usize,
    horizon_us: u64,
) -> Outcome {
    let url = format!("https://localhost:{}/live.ts", server.addr.port());
    let mut session = Session::open(&url, config, 0).unwrap();
    let mut now = 0u64;
    let mut out = Outcome {
        events: Vec::new(),
        requests: 0,
        offered: 0,
    };
    while now <= horizon_us && out.requests < attempts {
        while let Some(a) = session.poll(now) {
            if let Action::HttpRequest {
                conn,
                url,
                identity,
                ..
            } = a
            {
                out.requests += 1;
                out.offered += usize::from(identity.is_some());
                let path = url::Url::parse(&url).unwrap().path().to_string();
                match https_get(server.addr, &path, server_ca, identity.as_deref()) {
                    Ok(resp) => {
                        session.on_response(conn, resp.status, &resp.headers, now);
                        for part in resp.body.chunks(CHUNK) {
                            now += STEP_US;
                            session.on_data(conn, part, now);
                        }
                        session.on_closed(conn, None, now);
                    }
                    Err(e) => session.on_closed(conn, Some(e), now),
                }
            }
        }
        while let Some(e) = session.next_event() {
            out.events.push(e);
        }
        now += STEP_US * 100;
    }
    out
}

fn samples_of(events: &[Event]) -> Vec<SampleRec> {
    let replay = crate::support::replay::Replay {
        events: events
            .iter()
            .cloned()
            .map(|event| crate::support::replay::TimedEvent { at_us: 0, event })
            .collect(),
    };
    replay.samples()
}

#[test]
fn a_server_that_requires_a_client_certificate_is_played_with_a_pem_or_der_identity() {
    let ts = test_fixtures::read(SMALL);
    let want = baseline(&ts);
    let server_ca = Ca::new("server ca");
    let client_ca = Ca::new("client ca");
    let server = TlsServer::start(&server_ca, &client_ca, ts.clone());
    let leaf = client_ca.issue("player-1", true);
    for (name, identity) in [("pem", leaf.pem_identity()), ("der", leaf.der_identity())] {
        let config = cfg(|c| c.client_identity = Some(identity));
        let out = drive(&server, &server_ca, config, 1, 2_000_000);
        assert_eq!(out.offered, 1, "{name}");
        assert_eq!(samples_of(&out.events), want, "{name}");
    }
    let log = server.log.lock().unwrap();
    assert_eq!(log.handshake_failures, 0);
    assert_eq!(log.client_certs.len(), 2);
    for presented in &log.client_certs {
        assert_eq!(presented.as_deref(), Some(leaf.chain_der[0].as_slice()));
    }
}

#[test]
fn without_an_identity_the_handshake_fails_and_the_session_backs_off() {
    let server_ca = Ca::new("server ca");
    let client_ca = Ca::new("client ca");
    let ts = test_fixtures::read(SMALL);
    let server = TlsServer::start(&server_ca, &client_ca, ts);
    let out = drive(&server, &server_ca, Config::default(), 2, 30_000_000);
    assert_eq!(
        out.requests, 2,
        "a refused connection is retried after the backoff"
    );
    assert_eq!(out.offered, 0);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Reconnecting { .. })))
    );
    assert!(!out.events.iter().any(|e| matches!(e, Event::Sample { .. })));
    assert!(server.log.lock().unwrap().handshake_failures >= 1);
}

#[test]
fn a_certificate_from_the_wrong_ca_is_refused() {
    let server_ca = Ca::new("server ca");
    let client_ca = Ca::new("client ca");
    let other_ca = Ca::new("other ca");
    let ts = test_fixtures::read(SMALL);
    let server = TlsServer::start(&server_ca, &client_ca, ts);
    let stranger = other_ca.issue("stranger", true);
    let config = cfg(|c| c.client_identity = Some(stranger.pem_identity()));
    let out = drive(&server, &server_ca, config, 1, 1_000_000);
    assert_eq!(out.offered, 1);
    assert!(!out.events.iter().any(|e| matches!(e, Event::Sample { .. })));
    let log = server.log.lock().unwrap();
    assert_eq!(log.requests, 0);
    assert!(log.handshake_failures >= 1);
}

fn rtsps_redirect_identity(scope: ClientIdentityScope, to: &str) -> Option<u64> {
    let mut s = Session::open(
        "rtsps://a.example/live/",
        with_handle(|c| c.client_identity_scope = scope),
        0,
    )
    .unwrap();
    let mut conn = None;
    while let Some(a) = s.poll(0) {
        if let Action::TcpConnect { conn: c, .. } = a {
            conn = Some(c);
        }
    }
    let conn = conn.expect("a TcpConnect");
    s.on_connected(conn);
    while s.poll(0).is_some() {}
    let reply = format!("RTSP/1.0 301 Moved Permanently\r\nCSeq: 1\r\nLocation: {to}\r\n\r\n");
    s.on_data(conn, reply.as_bytes(), 1);
    let mut next = None;
    while let Some(a) = s.poll(1) {
        if let Action::TcpConnect { identity, tls, .. } = a {
            assert!(tls);
            next = Some(identity_handle(&identity));
        }
    }
    next.expect("the redirect target is connected")
}

#[test]
fn an_rtsps_redirect_to_another_host_gets_the_identity_only_under_any_https() {
    assert_eq!(
        rtsps_redirect_identity(
            ClientIdentityScope::SessionOrigin,
            "rtsps://b.example/live/"
        ),
        None
    );
    assert_eq!(
        rtsps_redirect_identity(ClientIdentityScope::AnyHttps, "rtsps://b.example/live/"),
        Some(HANDLE)
    );
    assert_eq!(
        rtsps_redirect_identity(
            ClientIdentityScope::SessionOrigin,
            "rtsps://a.example:8322/live/"
        ),
        None,
        "another port is another origin"
    );
    assert_eq!(
        rtsps_redirect_identity(
            ClientIdentityScope::SessionOrigin,
            "rtsps://a.example/other/"
        ),
        Some(HANDLE)
    );
}
