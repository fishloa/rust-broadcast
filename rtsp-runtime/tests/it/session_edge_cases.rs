//! Session edge cases: Transport selection, redirects, 454, stale auth, large reads, unknown methods.

use rtsp_runtime::{
    ClientEvent, ClientSession, ServerSession, SessionHeader, SessionState, Transport,
    TransportSpec,
};

fn wire(s: &str) -> Vec<u8> {
    s.replace('\n', "\r\n").into_bytes()
}

const BASE: &str = "rtsp://h/s";

fn client_in_ready() -> ClientSession {
    let mut c = ClientSession::new();
    let t = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
    c.setup(BASE, &t).unwrap();
    c.handle_data_strict(&wire(
        "RTSP/1.0 200 OK\nCSeq: 1\nSession: abc\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\n\n",
    ))
    .unwrap();
    assert_eq!(c.state(), SessionState::Ready);
    c
}

#[test]
fn server_picks_the_first_supported_transport_spec_instead_of_answering_461() {
    let mut s = ServerSession::new(|| 7);
    let req = wire(
        "SETUP rtsp://h/s/t RTSP/1.0\nCSeq: 1\nTransport: RTP/SAVP;unicast;client_port=5000-5001, RTP/AVP;unicast;client_port=5002-5003\n\n",
    );
    let (resp, _) = s.handle_request(&req).unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.starts_with("RTSP/1.0 200"), "got: {text}");
}

#[test]
fn session_timeout_above_u32_is_kept_not_replaced_by_the_default() {
    let h = SessionHeader::parse("abc;timeout=4294967296").unwrap();
    assert_eq!(
        h.timeout,
        Some(std::time::Duration::from_secs(4_294_967_296))
    );
}

#[test]
fn failed_setup_response_does_not_record_a_negotiated_transport() {
    let mut c = ClientSession::new();
    let t = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
    c.setup(BASE, &t).unwrap();
    c.handle_data_strict(&wire(
        "RTSP/1.0 461 Unsupported Transport\nCSeq: 1\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\n\n",
    ))
    .unwrap();
    assert!(c.negotiated_transport().is_none());
}

#[test]
fn setup_2xx_with_an_unmodelled_transport_profile_still_delivers_the_response() {
    let mut c = ClientSession::new();
    let t = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
    c.setup(BASE, &t).unwrap();
    let out = c.handle_data(&wire(
        "RTSP/1.0 200 OK\nCSeq: 1\nSession: abc\nTransport: RTP/AVPF/UDP;unicast;client_port=5000-5001\n\n",
    ));
    assert!(out.error.is_none(), "terminal error: {:?}", out.error);
    assert_eq!(c.state(), SessionState::Ready);
}

#[test]
fn redirect_resets_state_to_init_and_forgets_the_old_session_id() {
    let mut c = client_in_ready();
    c.play(BASE).unwrap();
    c.handle_data_strict(&wire(
        "RTSP/1.0 302 Found\nCSeq: 2\nLocation: rtsp://other/s\n\n",
    ))
    .unwrap();
    assert_eq!(c.state(), SessionState::Init);
    let t = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
    let setup = String::from_utf8(c.setup("rtsp://other/s", &t).unwrap()).unwrap();
    assert!(
        !setup.contains("Session:"),
        "stale Session id sent to the redirect target:\n{setup}"
    );
}

#[test]
fn teardown_answered_454_leaves_the_client_in_init() {
    let mut c = client_in_ready();
    c.teardown(BASE).unwrap();
    c.handle_data_strict(&wire("RTSP/1.0 454 Session Not Found\nCSeq: 2\n\n"))
        .unwrap();
    assert_eq!(c.state(), SessionState::Init);
    assert_eq!(c.session_id(), None);
}

#[test]
fn a_large_read_of_valid_interleaved_frames_is_not_a_terminal_error() {
    let mut c = ClientSession::new();
    let frame = {
        let mut f = vec![b'$', 0, 0xFF, 0xFF];
        f.extend(std::iter::repeat_n(0u8, 0xFFFF));
        f
    };
    let data: Vec<u8> = std::iter::repeat_n(frame, 20).flatten().collect();
    assert!(data.len() > 1024 * 1024);
    let out = c.handle_data(&data);
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(out.events.len(), 20);
    assert!(matches!(out.events[0], ClientEvent::MediaData { .. }));
}

#[test]
fn endless_stale_401_challenges_end_in_the_401_response() {
    let mut c = ClientSession::new().with_credentials(rtsp_runtime::Credentials::new("u", "p"));
    c.options(BASE).unwrap();
    let mut cseq = 1;
    let mut gave_up = false;
    for _ in 0..50 {
        let out = c.handle_data_strict(&wire(&format!(
            "RTSP/1.0 401 Unauthorized\nCSeq: {cseq}\nWWW-Authenticate: Digest realm=\"r\", nonce=\"n{cseq}\", qop=\"auth\", stale=true\n\n"
        ))).unwrap();
        match &out[0] {
            ClientEvent::AuthRetry { cseq: n, .. } => cseq = *n,
            ClientEvent::Response { status, .. } => {
                assert_eq!(*status, rtsp_runtime::StatusCode::Unauthorized);
                gave_up = true;
                break;
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(
        gave_up,
        "client kept re-sending after 50 consecutive stale=true 401s"
    );
}

#[test]
fn a_session_timeout_beyond_the_u64_digit_bound_is_a_warned_default_not_a_silent_one() {
    let (h, warnings) =
        SessionHeader::parse_with_warnings("abc;timeout=99999999999999999999").unwrap();
    assert_eq!(h.timeout, Some(std::time::Duration::from_secs(60)));
    assert_eq!(warnings.len(), 1);
}

#[test]
fn a_request_without_cseq_is_answered_400_and_the_session_continues() {
    let mut s = ServerSession::new(|| 7);
    let (resp, _) = s
        .handle_request(&wire("OPTIONS rtsp://h/s RTSP/1.0\n\n"))
        .unwrap();
    assert!(String::from_utf8_lossy(&resp).starts_with("RTSP/1.0 400"));
    let (resp, _) = s
        .handle_request(&wire("OPTIONS rtsp://h/s RTSP/1.0\nCSeq: 1\n\n"))
        .unwrap();
    assert!(String::from_utf8_lossy(&resp).starts_with("RTSP/1.0 200"));
}

#[test]
fn an_unknown_method_is_answered_501() {
    let mut s = ServerSession::new(|| 7);
    let (resp, _) = s
        .handle_request(&wire("FROBNICATE rtsp://h/s RTSP/1.0\nCSeq: 1\n\n"))
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.starts_with("RTSP/1.0 501"), "got: {text}");
}
