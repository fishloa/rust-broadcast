//! Bug audit B reproductions. Each test asserts the correct behaviour and fails on main.

use http::{HeaderMap, HeaderValue, StatusCode, header};
use webrtc_runtime::whip::client::{self, WhipClient};
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
fn trickle_ack_is_not_mistaken_for_the_delete_ack_when_both_are_in_flight() {
    let mut c = established_client();
    c.flush_candidates(FRAG.to_vec()).unwrap();
    let del = c.terminate();
    if del.is_err() {
        return;
    }
    let ev = c.on_response(client::HttpResponse::new(StatusCode::NO_CONTENT));
    assert!(
        !matches!(ev, Ok(Some(client::Event::Terminated))),
        "the trickle PATCH's 204 closed the session before the DELETE was answered"
    );
}

#[test]
fn offer_can_be_retried_after_a_503() {
    let mut c = WhipClient::new(ENDPOINT.into(), None);
    c.offer(b"v=0\r\n".to_vec()).unwrap();
    assert!(
        c.on_response(client::HttpResponse::new(StatusCode::SERVICE_UNAVAILABLE))
            .is_err()
    );
    assert!(c.offer(b"v=0\r\n".to_vec()).is_ok(), "client is stuck in offer-sent");
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
    s.ack_restart(FRAG.to_vec(), "e2".into());
    assert_eq!(*s.state(), server::State::Closed);
}

#[test]
fn accept_after_close_does_not_reopen_the_session() {
    let mut s = WhipSession::new(SESSION.into());
    s.on_post(b"v=0\r\n".to_vec()).unwrap();
    s.accept(b"v=0\r\n".to_vec(), "e1".into());
    s.on_delete().unwrap();
    s.accept(b"v=0\r\n".to_vec(), "e1".into());
    assert_eq!(*s.state(), server::State::Closed);
}
