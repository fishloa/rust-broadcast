//! Session state-machine invariants: request correlation, retry after an error, closed sessions.

use http::{HeaderMap, HeaderValue, StatusCode, header};
use webrtc_runtime::whip::client::{self, WhipClient};
use webrtc_runtime::whep::server::WhepSession;
use webrtc_runtime::whip::server::{self, WhipSession};

const ENDPOINT: &str = "https://whip.example.com/pub";
const SESSION: &str = "https://whip.example.com/session/abc";
const FRAG: &[u8] = b"a=ice-ufrag:t\r\na=ice-pwd:t\r\n";

fn established_client() -> WhipClient {
    let mut c = WhipClient::new(ENDPOINT.into(), None);
    c.offer(b"v=0\r\n".to_vec()).unwrap();
    c.on_response(
        client::HttpResponse::new(StatusCode::CREATED)
            .with_location(SESSION)
            .unwrap()
            .with_etag("e1")
            .unwrap(),
    )
    .unwrap();
    c
}

fn trickle_ct() -> HeaderMap {
    let mut m = HeaderMap::new();
    m.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/trickle-ice-sdpfrag"),
    );
    m
}

#[test]
fn a_second_request_is_refused_while_one_is_in_flight() {
    let mut c = established_client();
    c.flush_candidates(FRAG.to_vec()).unwrap();
    assert!(matches!(
        c.terminate(),
        Err(webrtc_runtime::Error::WrongState { .. })
    ));
    assert!(matches!(
        c.ice_restart(FRAG.to_vec()),
        Err(webrtc_runtime::Error::WrongState { .. })
    ));
    let ev = c
        .on_response(client::HttpResponse::new(StatusCode::NO_CONTENT))
        .unwrap();
    assert!(ev.is_none());
    assert!(matches!(c.state(), client::State::Established { .. }));
    c.terminate().unwrap();
    let ev = c
        .on_response(client::HttpResponse::new(StatusCode::NO_CONTENT))
        .unwrap();
    assert!(matches!(ev, Some(client::Event::Terminated)));
}

#[test]
fn offer_can_be_retried_after_a_503_or_a_redirect() {
    for status in [StatusCode::SERVICE_UNAVAILABLE, StatusCode::TEMPORARY_REDIRECT] {
        let mut c = WhipClient::new(ENDPOINT.into(), None);
        c.offer(b"v=0\r\n".to_vec()).unwrap();
        assert!(c.on_response(client::HttpResponse::new(status)).is_err());
        assert_eq!(*c.state(), client::State::Idle);
        assert!(c.offer(b"v=0\r\n".to_vec()).is_ok(), "{status}");
    }
}

#[test]
fn late_ack_restart_does_not_resurrect_a_deleted_session() {
    let mut s = WhipSession::new(SESSION.into());
    s.on_post(b"v=0\r\n".to_vec()).unwrap();
    s.accept(b"v=0\r\n".to_vec(), "e1".into());
    let mut h = trickle_ct();
    h.insert(header::IF_MATCH, HeaderValue::from_static("*"));
    s.on_patch(FRAG.to_vec(), &h).unwrap();
    s.on_delete().unwrap();
    assert_eq!(*s.state(), server::State::Closed);
    let resp = s.ack_restart(FRAG.to_vec(), "e2".into());
    assert_eq!(resp.status, StatusCode::NOT_FOUND);
    assert_eq!(*s.state(), server::State::Closed);
}

#[test]
fn accept_after_close_does_not_reopen_the_session() {
    let mut s = WhipSession::new(SESSION.into());
    s.on_post(b"v=0\r\n".to_vec()).unwrap();
    s.accept(b"v=0\r\n".to_vec(), "e1".into());
    s.on_delete().unwrap();
    let resp = s.accept(b"v=0\r\n".to_vec(), "e1".into());
    assert_eq!(resp.status, StatusCode::NOT_FOUND);
    assert_eq!(*s.state(), server::State::Closed);
}

#[test]
fn ack_answer_outside_a_counter_offer_is_refused() {
    let mut s = WhepSession::new(SESSION.into());
    let resp = s.ack_answer("e1".into());
    assert_eq!(resp.status, StatusCode::NOT_FOUND);
    assert_eq!(*s.state(), server::State::AwaitingOffer);

    s.on_post(b"v=0\r\n".to_vec()).unwrap();
    s.counter_offer(b"v=0\r\n".to_vec(), None);
    s.on_delete().unwrap();
    let resp = s.ack_answer("e1".into());
    assert_eq!(resp.status, StatusCode::NOT_FOUND);
    assert_eq!(*s.state(), server::State::Closed);
}

#[test]
fn ack_answer_validates_the_etag_and_sends_it() {
    let mut s = WhepSession::new(SESSION.into());
    s.on_post(b"v=0\r\n".to_vec()).unwrap();
    s.counter_offer(b"v=0\r\n".to_vec(), None);
    let resp = s.ack_answer("bad\"tag".into());
    assert_eq!(resp.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(s.state(), server::State::CounterOffered { .. }));

    let resp = s.ack_answer("e1".into());
    assert_eq!(resp.status, StatusCode::NO_CONTENT);
    assert!(resp.headers.contains_key(header::ETAG));
    assert_eq!(*s.state(), server::State::Established { etag: "e1".into() });
}
