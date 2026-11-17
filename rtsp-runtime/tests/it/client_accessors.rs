//! #2945: the client accessors that feed keepalive scheduling, transport
//! selection and media synchronisation must return what the server actually
//! sent, and the capture conditions must never fire on an unsuccessful
//! response.

use rtsp_runtime::{ClientEvent, ClientSession, Credentials, Method, SessionState, StatusCode};
use rtsp_runtime::{Transport, TransportSpec};

const BASE: &str = "rtsp://h/s";

fn wire(s: &str) -> Vec<u8> {
    s.replace('\n', "\r\n").into_bytes()
}

fn tcp_transport() -> Transport {
    Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1))
}

#[test]
fn the_options_response_pins_public_methods_without_leaking_play_or_describe_state() {
    let mut c = ClientSession::default();
    c.options(BASE).expect("OPTIONS from Init");
    c.handle_data_strict(&wire(
        "RTSP/1.0 200 OK\nCSeq: 1\nPublic: DESCRIBE, SETUP, PLAY\n\
         Content-Base: rtsp://h/base/\n\
         RTP-Info: url=rtsp://h/s/trackID=0;seq=7;rtptime=9000\n\n",
    ))
    .expect("OPTIONS response");

    assert_eq!(
        c.public_methods(),
        ["DESCRIBE", "SETUP", "PLAY"],
        "the server's advertised methods, in wire order"
    );
    assert_eq!(c.keepalive_method(), Method::Options);
    assert_eq!(
        c.content_base(),
        None,
        "Content-Base comes only from a successful DESCRIBE"
    );
    assert!(
        c.take_rtp_info().is_empty(),
        "RTP-Info comes only from a successful PLAY"
    );
}

#[test]
fn a_failed_describe_does_not_capture_content_base() {
    let mut c = ClientSession::default();
    c.describe(BASE).expect("DESCRIBE from Init");
    c.handle_data_strict(&wire(
        "RTSP/1.0 404 Not Found\nCSeq: 1\nContent-Base: rtsp://h/base/\n\n",
    ))
    .expect("404 response");
    assert_eq!(c.content_base(), None);
}

#[test]
fn a_successful_describe_captures_content_base() {
    let mut c = ClientSession::default();
    c.describe(BASE).expect("DESCRIBE from Init");
    c.handle_data_strict(&wire(
        "RTSP/1.0 200 OK\nCSeq: 1\nContent-Type: application/sdp\n\
         Content-Base: rtsp://h/base/\nContent-Length: 5\n\nv=0\n",
    ))
    .expect("DESCRIBE response");
    assert_eq!(c.content_base(), Some("rtsp://h/base/"));
}

#[test]
fn the_setup_response_exposes_its_session_timeout_and_transport() {
    let mut c = ClientSession::default();
    c.setup(BASE, &tcp_transport()).expect("SETUP");
    c.handle_data_strict(&wire(
        "RTSP/1.0 200 OK\nCSeq: 1\nSession: abc123;timeout=45\n\
         Transport: RTP/AVP/TCP;unicast;interleaved=0-1\n\n",
    ))
    .expect("SETUP response");

    assert_eq!(c.session_timeout(), Some(45));
    assert_eq!(c.negotiated_transport(), Some(&tcp_transport()));
    assert_eq!(c.state(), SessionState::Ready);
}

#[test]
fn a_setup_response_without_a_timeout_reports_none() {
    let mut c = ClientSession::default();
    c.setup(BASE, &tcp_transport()).expect("SETUP");
    c.handle_data_strict(&wire(
        "RTSP/1.0 200 OK\nCSeq: 1\nSession: abc123\n\
         Transport: RTP/AVP/TCP;unicast;interleaved=0-1\n\n",
    ))
    .expect("SETUP response");
    assert_eq!(c.session_timeout(), None);
}

fn client_in_ready() -> ClientSession {
    let mut c = ClientSession::default();
    c.setup(BASE, &tcp_transport()).expect("SETUP");
    c.handle_data_strict(&wire(
        "RTSP/1.0 200 OK\nCSeq: 1\nSession: abc123\n\
         Transport: RTP/AVP/TCP;unicast;interleaved=0-1\n\n",
    ))
    .expect("SETUP response");
    assert_eq!(c.state(), SessionState::Ready);
    c
}

#[test]
fn a_failed_play_does_not_capture_rtp_info() {
    let mut c = client_in_ready();
    c.play(BASE).expect("PLAY from Ready");
    c.handle_data_strict(&wire(
        "RTSP/1.0 454 Session Not Found\nCSeq: 2\n\
         RTP-Info: url=rtsp://h/s/trackID=0;seq=7;rtptime=9000\n\n",
    ))
    .expect("454 response");
    assert!(c.take_rtp_info().is_empty());
    assert_eq!(c.state(), SessionState::Init);
}

#[test]
fn a_successful_play_captures_rtp_info() {
    let mut c = client_in_ready();
    c.play(BASE).expect("PLAY from Ready");
    c.handle_data_strict(&wire(
        "RTSP/1.0 200 OK\nCSeq: 2\n\
         RTP-Info: url=rtsp://h/s/trackID=0;seq=7;rtptime=9000\n\n",
    ))
    .expect("PLAY response");
    let info = c.take_rtp_info();
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].seq, Some(7));
    assert_eq!(info[0].rtptime, Some(9000));
    assert!(c.take_rtp_info().is_empty(), "taken once");
}

const CHALLENGE: &str = "WWW-Authenticate: Digest realm=\"r\", nonce=\"n1\", qop=\"auth\"\n\n";

fn unauthorized(cseq: u32, challenge: &str) -> Vec<u8> {
    wire(&format!(
        "RTSP/1.0 401 Unauthorized\nCSeq: {cseq}\n{challenge}"
    ))
}

#[test]
fn a_second_401_without_stale_does_not_retry_again() {
    let mut c = ClientSession::default().with_credentials(Credentials::new("u", "p"));
    c.options(BASE).expect("OPTIONS from Init");

    let first = c
        .handle_data_strict(&unauthorized(1, CHALLENGE))
        .expect("first 401");
    let retry_cseq = match &first[..] {
        [ClientEvent::AuthRetry { cseq, .. }] => *cseq,
        other => panic!("expected one AuthRetry, got {other:?}"),
    };

    let second = c
        .handle_data_strict(&unauthorized(retry_cseq, CHALLENGE))
        .expect("second 401");
    assert!(
        matches!(
            &second[..],
            [ClientEvent::Response { status, .. }] if *status == StatusCode::Unauthorized
        ),
        "a repeated 401 with no stale=true must reach the caller: {second:?}"
    );
}

#[test]
fn stale_401s_retry_exactly_three_times_then_reach_the_caller() {
    let mut c = ClientSession::default().with_credentials(Credentials::new("u", "p"));
    c.options(BASE).expect("OPTIONS from Init");

    let mut cseq = 1;
    let mut retries = 0;
    let mut final_status = None;
    for nonce in 0..10 {
        let out = c
            .handle_data_strict(&unauthorized(
                cseq,
                &format!(
                    "WWW-Authenticate: Digest realm=\"r\", nonce=\"n{nonce}\", qop=\"auth\", stale=true\n\n"
                ),
            ))
            .expect("stale 401");
        match &out[..] {
            [ClientEvent::AuthRetry { cseq: next, .. }] => {
                cseq = *next;
                retries += 1;
            }
            [ClientEvent::Response { status, .. }] => {
                final_status = Some(*status);
                break;
            }
            other => panic!("unexpected events {other:?}"),
        }
    }
    assert_eq!(retries, 3, "stale=true earns a bounded number of replays");
    assert_eq!(final_status, Some(StatusCode::Unauthorized));
}
