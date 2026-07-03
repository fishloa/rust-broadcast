//! Integration tests for the sans-IO RTSP engine — the exit gate for issue #521.
//!
//! Each test drives the engine end-to-end against the RFC 2326 fixtures in
//! `tests/fixtures/` and asserts observable behaviour that a no-op / passthrough
//! implementation would fail (each is annotated with how it "bites").

use rtsp_runtime::client::ClientEvent;
use rtsp_runtime::server::ServerEvent;
use rtsp_runtime::{
    ClientSession, InterleavedFrame, Method, ServerSession, SessionState, StatusCode, Transport,
    TransportSpec,
};

/// Builds RTSP wire bytes from a `\n`-delimited string, converting to CRLF.
fn wire(s: &str) -> Vec<u8> {
    s.replace('\n', "\r\n").into_bytes()
}

// ---------------------------------------------------------------------------
// Gate 1 — Client full-session replay (session_tcp_interleaved.md).
// ---------------------------------------------------------------------------

#[test]
fn gate1_client_full_session_replay() {
    let base = "rtsp://video.example.com/stream";
    let track = "rtsp://video.example.com/stream/trackID=0";
    let mut c = ClientSession::new();

    // OPTIONS (CSeq 1) — state-neutral.
    let out = c.options(base).unwrap();
    assert!(cseq_line(&out, 1), "OPTIONS must carry CSeq: 1");
    let ev = c
        .handle_data(&wire(
            "RTSP/1.0 200 OK\nCSeq: 1\nPublic: OPTIONS, DESCRIBE, SETUP, PLAY, PAUSE, TEARDOWN\n\n",
        ))
        .unwrap();
    assert_response(&ev, 1, Method::Options, StatusCode::Ok);
    assert_eq!(c.state(), SessionState::Init);

    // DESCRIBE (CSeq 2).
    let out = c.describe(base).unwrap();
    assert!(cseq_line(&out, 2));
    let ev = c
        .handle_data(&wire(
            "RTSP/1.0 200 OK\nCSeq: 2\nContent-Type: application/sdp\nContent-Length: 53\n\nv=0\no=- 1 1 IN IP4 192.0.2.1\nm=video 0 RTP/AVP 96\n",
        ))
        .unwrap();
    assert_response(&ev, 2, Method::Describe, StatusCode::Ok);

    // SETUP (CSeq 3) — Transport TCP interleaved.
    let t = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
    let out = c.setup(track, &t).unwrap();
    assert!(cseq_line(&out, 3));
    assert!(
        String::from_utf8_lossy(&out).contains("interleaved=0-1"),
        "SETUP must carry the interleaved transport"
    );
    let ev = c
        .handle_data(&wire(
            "RTSP/1.0 200 OK\nCSeq: 3\nSession: 12345678\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\n\n",
        ))
        .unwrap();
    assert_response(&ev, 3, Method::Setup, StatusCode::Ok);
    assert_eq!(c.state(), SessionState::Ready, "SETUP 2xx -> Ready");
    assert_eq!(c.session_id(), Some("12345678"));

    // PLAY (CSeq 4) — must carry the session id captured from SETUP.
    let out = c.play(base).unwrap();
    assert!(cseq_line(&out, 4));
    assert!(
        String::from_utf8_lossy(&out).contains("Session: 12345678"),
        "PLAY must echo the SETUP session id"
    );
    let ev = c
        .handle_data(&wire(
            "RTSP/1.0 200 OK\nCSeq: 4\nSession: 12345678\nRTP-Info: url=rtsp://video.example.com/stream/trackID=0;seq=10001;rtptime=0\n\n",
        ))
        .unwrap();
    assert_response(&ev, 4, Method::Play, StatusCode::Ok);
    assert_eq!(c.state(), SessionState::Playing, "PLAY 2xx -> Playing");

    // TEARDOWN (CSeq 5) — must carry the session id.
    let out = c.teardown(base).unwrap();
    assert!(cseq_line(&out, 5));
    assert!(String::from_utf8_lossy(&out).contains("Session: 12345678"));
    let ev = c
        .handle_data(&wire("RTSP/1.0 200 OK\nCSeq: 5\n\n"))
        .unwrap();
    assert_response(&ev, 5, Method::Teardown, StatusCode::Ok);
    assert_eq!(c.state(), SessionState::Init, "TEARDOWN 2xx -> Init");
}

// ---------------------------------------------------------------------------
// Gate 2 — Illegal-method-in-state bites.
// ---------------------------------------------------------------------------

#[test]
fn gate2_illegal_method_in_state_bites() {
    // play() in Init is rejected before any bytes are produced.
    let mut c = ClientSession::new();
    assert!(
        c.play("rtsp://h/s").is_err(),
        "PLAY in Init must be rejected"
    );

    // Drive to Ready via SETUP, then pause() must be rejected.
    let t = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
    c.setup("rtsp://h/s", &t).unwrap();
    c.handle_data(&wire(
        "RTSP/1.0 200 OK\nCSeq: 1\nSession: 42\nTransport: RTP/AVP/TCP;interleaved=0-1\n\n",
    ))
    .unwrap();
    assert_eq!(c.state(), SessionState::Ready);
    assert!(
        c.pause("rtsp://h/s").is_err(),
        "PAUSE in Ready must be rejected"
    );

    // setup() is allowed in both Init and Ready.
    let mut c2 = ClientSession::new();
    assert!(c2.setup("rtsp://h/s", &t).is_ok(), "SETUP allowed in Init");
    c2.handle_data(&wire(
        "RTSP/1.0 200 OK\nCSeq: 1\nSession: 42\nTransport: RTP/AVP/TCP;interleaved=0-1\n\n",
    ))
    .unwrap();
    assert_eq!(c2.state(), SessionState::Ready);
    assert!(c2.setup("rtsp://h/s", &t).is_ok(), "SETUP allowed in Ready");
}

// ---------------------------------------------------------------------------
// Gate 3 — Digest auth bites (digest_auth.md).
// ---------------------------------------------------------------------------

#[test]
fn gate3_digest_auth_bites() {
    use rtsp_runtime::Credentials;

    let uri = "rtsp://camera.example.com/live";
    let mut c = ClientSession::new().with_credentials(Credentials::new("admin", "12345"));

    // Initial DESCRIBE (CSeq 1), no Authorization.
    let out = c.describe(uri).unwrap();
    assert!(!String::from_utf8_lossy(&out).contains("Authorization:"));

    // Feed the 401 Digest challenge -> engine emits a NEW authed DESCRIBE.
    let events = c
        .handle_data(&wire(
            "RTSP/1.0 401 Unauthorized\nCSeq: 1\nWWW-Authenticate: Digest realm=\"IP Camera\",nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\",qop=\"auth\",algorithm=MD5\n\n",
        ))
        .unwrap();
    let retry = events
        .iter()
        .find_map(|e| match e {
            ClientEvent::AuthRetry { request, cseq, .. } => Some((request.clone(), *cseq)),
            _ => None,
        })
        .expect("a 401 must trigger an AuthRetry event");
    let (retry_bytes, retry_cseq) = retry;
    assert_eq!(retry_cseq, 2, "retry uses a fresh CSeq");

    let text = String::from_utf8_lossy(&retry_bytes);
    // Extract the Authorization header value.
    let auth_line = text
        .lines()
        .find(|l| l.starts_with("Authorization:"))
        .expect("retry must carry an Authorization header");
    let value = auth_line.trim_start_matches("Authorization:").trim();
    assert!(value.starts_with("Digest "), "got: {value}");
    for needle in ["response=", "realm=", "nonce=", "uri=", "cnonce=", "nc="] {
        assert!(
            value.contains(needle),
            "Authorization missing {needle}: {value}"
        );
    }
    // The response must be a real computed digest, not empty and not the
    // illustrative fixture value echoed back.
    assert!(!value.contains("response=\"\""), "empty digest response");
    assert!(
        !value.contains("6629fae49393a05397450978507c4ef1"),
        "digest was echoed from the fixture, not computed"
    );
    assert!(value.contains("uri=\"rtsp://camera.example.com/live\""));

    // Feed the 200 -> DESCRIBE completes.
    let events = c
        .handle_data(&wire(
            "RTSP/1.0 200 OK\nCSeq: 2\nContent-Type: application/sdp\nContent-Length: 5\n\nv=0\n",
        ))
        .unwrap();
    assert_response(&events, 2, Method::Describe, StatusCode::Ok);
}

#[test]
fn gate3b_stale_rechallenge_refreshes_nonce() {
    use rtsp_runtime::Credentials;

    let uri = "rtsp://camera.example.com/live";
    let mut c = ClientSession::new().with_credentials(Credentials::new("admin", "12345"));
    c.describe(uri).unwrap();

    // First 401.
    let ev1 = c
        .handle_data(&wire(
            "RTSP/1.0 401 Unauthorized\nCSeq: 1\nWWW-Authenticate: Digest realm=\"IP Camera\",nonce=\"AAAA0000\",qop=\"auth\",algorithm=MD5\n\n",
        ))
        .unwrap();
    let first = auth_response_value(&ev1);
    assert!(first.contains("nonce=\"AAAA0000\""));

    // Stale re-challenge with a NEW nonce -> retry must use the new nonce.
    let ev2 = c
        .handle_data(&wire(
            "RTSP/1.0 401 Unauthorized\nCSeq: 2\nWWW-Authenticate: Digest realm=\"IP Camera\",nonce=\"BBBB1111\",qop=\"auth\",stale=true,algorithm=MD5\n\n",
        ))
        .unwrap();
    let second = auth_response_value(&ev2);
    assert!(
        second.contains("nonce=\"BBBB1111\""),
        "stale re-challenge must refresh the nonce: {second}"
    );
}

// ---------------------------------------------------------------------------
// Gate 4 — Interleaved framing bites.
// ---------------------------------------------------------------------------

#[test]
fn gate4_interleaved_framing_bites() {
    // Build -> serialize -> parse -> equal.
    let frame = InterleavedFrame::new(0, (0u8..37).collect::<Vec<u8>>());
    let bytes = frame.to_bytes().unwrap();
    let (parsed, consumed) = InterleavedFrame::parse(&bytes).unwrap().unwrap();
    assert_eq!(parsed, frame);
    assert_eq!(consumed, bytes.len());

    // Two full frames + a partial third -> exactly 2 frames + partial remainder.
    let f0 = InterleavedFrame::new(0, vec![0x11; 20]);
    let f1 = InterleavedFrame::new(1, vec![0x22; 8]);
    let mut buf = Vec::new();
    buf.extend_from_slice(&f0.to_bytes().unwrap());
    buf.extend_from_slice(&f1.to_bytes().unwrap());
    // Partial: header says 12 payload bytes, only 5 present.
    let partial = [0x24u8, 0x00, 0x00, 0x0C, 9, 8, 7, 6, 5];
    buf.extend_from_slice(&partial);

    let (frames, remainder) = rtsp_runtime::interleaved::parse_frames(&buf).unwrap();
    assert_eq!(frames.len(), 2, "must not consume the partial frame");
    assert_eq!(frames[0], f0);
    assert_eq!(frames[1], f1);
    assert_eq!(remainder, partial.len());
    assert_eq!(&buf[buf.len() - remainder..], &partial);
}

#[test]
fn gate4b_client_emits_media_data_for_interleaved() {
    let mut c = ClientSession::new();
    let f = InterleavedFrame::new(0, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    let events = c.handle_data(&f.to_bytes().unwrap()).unwrap();
    assert_eq!(
        events,
        vec![ClientEvent::MediaData {
            channel: 0,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
        }]
    );
}

// ---------------------------------------------------------------------------
// Gate 5 — Transport round-trip bites.
// ---------------------------------------------------------------------------

#[test]
fn gate5_transport_round_trip_bites() {
    let a = Transport::parse("RTP/AVP/TCP;interleaved=0-1").unwrap();
    let a2 = Transport::parse(&a.to_header_value()).unwrap();
    assert_eq!(a, a2);
    assert_eq!(a.first().unwrap().interleaved, Some((0, 1)));

    let b = Transport::parse("RTP/AVP;unicast;client_port=8000-8001").unwrap();
    let b2 = Transport::parse(&b.to_header_value()).unwrap();
    assert_eq!(b, b2);
    assert_eq!(b.first().unwrap().client_port, Some((8000, 8001)));
}

// ---------------------------------------------------------------------------
// Gate 6 — Server transition bites.
// ---------------------------------------------------------------------------

#[test]
fn gate6_server_transitions_bite() {
    let mut s = ServerSession::new();

    // SETUP -> 200 with Session + Transport, Init -> Ready.
    let (resp, events) = s
        .handle_request(&wire(
            "SETUP rtsp://h/s/trackID=0 RTSP/1.0\nCSeq: 1\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\n\n",
        ))
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.starts_with("RTSP/1.0 200"));
    assert!(text.contains("Session:"));
    assert!(text.contains("Transport:"));
    assert!(text.contains("interleaved=0-1"));
    assert_eq!(s.state(), SessionState::Ready);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ServerEvent::SessionSetup { .. }))
    );
    let sid = s.session_id().unwrap().to_string();

    // PLAY -> Playing.
    let (resp, _) = s
        .handle_request(&wire(&format!(
            "PLAY rtsp://h/s RTSP/1.0\nCSeq: 2\nSession: {sid}\n\n"
        )))
        .unwrap();
    assert!(String::from_utf8_lossy(&resp).starts_with("RTSP/1.0 200"));
    assert_eq!(s.state(), SessionState::Playing);

    // Fresh session: PLAY in Init -> 455, state unchanged.
    let mut fresh = ServerSession::new();
    let (resp, events) = fresh
        .handle_request(&wire("PLAY rtsp://h/s RTSP/1.0\nCSeq: 1\n\n"))
        .unwrap();
    assert!(String::from_utf8_lossy(&resp).contains("455"));
    assert_eq!(
        fresh.state(),
        SessionState::Init,
        "455 must not change state"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ServerEvent::MethodNotValid { .. }))
    );
}

// ---------------------------------------------------------------------------
// Gate 7 — SDP parse (proves the DESCRIBE body path).
// ---------------------------------------------------------------------------

#[test]
fn gate7_sdp_parse() {
    let sdp = std::fs::read("tests/fixtures/sdp_sample.sdp").expect("read sdp fixture");
    let session = sdp_types::Session::parse(&sdp).expect("parse SDP");
    assert_eq!(session.medias.len(), 2, "two media sections (video+audio)");
    assert_eq!(session.medias[0].media, "video");
    assert_eq!(session.medias[1].media, "audio");

    // a=control attributes are read (aggregate + per-stream).
    assert_eq!(
        session.get_first_attribute_value("control").unwrap(),
        Some("rtsp://media.example.com/movie")
    );
    assert_eq!(
        session.medias[0]
            .get_first_attribute_value("control")
            .unwrap(),
        Some("trackID=1")
    );
    assert_eq!(
        session.medias[1]
            .get_first_attribute_value("control")
            .unwrap(),
        Some("trackID=2")
    );
}

// --- helpers ---------------------------------------------------------------

fn cseq_line(bytes: &[u8], n: u32) -> bool {
    String::from_utf8_lossy(bytes).contains(&format!("CSeq: {n}"))
}

fn assert_response(events: &[ClientEvent], cseq: u32, method: Method, status: StatusCode) {
    let found = events.iter().any(|e| {
        matches!(
            e,
            ClientEvent::Response { cseq: c, method: m, status: s, .. }
                if *c == cseq && *m == method && *s == status
        )
    });
    assert!(
        found,
        "expected Response cseq={cseq} {method:?} {status:?}, got {events:?}"
    );
}

fn auth_response_value(events: &[ClientEvent]) -> String {
    let bytes = events
        .iter()
        .find_map(|e| match e {
            ClientEvent::AuthRetry { request, .. } => Some(request.clone()),
            _ => None,
        })
        .expect("expected an AuthRetry event");
    let text = String::from_utf8_lossy(&bytes);
    text.lines()
        .find(|l| l.starts_with("Authorization:"))
        .expect("Authorization header")
        .trim_start_matches("Authorization:")
        .trim()
        .to_string()
}
