//! Session state-machine invariants: request correlation, retry after an error, closed sessions.

use http::{HeaderMap, HeaderValue, StatusCode, header};
use webrtc_runtime::whep::server::WhepSession;
use webrtc_runtime::whip::client::{self, WhipClient};
use webrtc_runtime::whip::server::{self, WhipSession};

/// The endpoint URL as the parsed value the state machines take.
fn endpoint() -> url::Url {
    url::Url::parse(ENDPOINT).expect("endpoint")
}

fn session_url() -> url::Url {
    url::Url::parse(SESSION).expect("session URL")
}

const ENDPOINT: &str = "https://whip.example.com/pub";
const SESSION: &str = "https://whip.example.com/session/abc";
const FRAG: &[u8] = b"a=ice-ufrag:t\r\na=ice-pwd:t\r\n";

fn established_client() -> WhipClient {
    let mut c = WhipClient::new(webrtc_runtime::session::client::WebRtcClientConfig::new(
        endpoint(),
    ));
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
fn responses_are_matched_to_requests_in_the_order_they_were_sent() {
    let mut c = established_client();
    c.flush_candidates(FRAG.to_vec()).unwrap();
    c.terminate().unwrap();
    let ev = c
        .on_response(client::HttpResponse::new(StatusCode::NO_CONTENT))
        .unwrap();
    assert!(ev.is_none(), "the trickle 204 must not close the session");
    assert!(matches!(c.state(), client::State::Established { .. }));
    let ev = c
        .on_response(client::HttpResponse::new(StatusCode::NO_CONTENT))
        .unwrap();
    assert!(matches!(ev, Some(client::Event::Terminated)));
    assert_eq!(*c.state(), client::State::Closed);
}

#[test]
fn offer_can_be_retried_after_a_503_or_a_redirect() {
    for status in [
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::TEMPORARY_REDIRECT,
    ] {
        let mut c = WhipClient::new(webrtc_runtime::session::client::WebRtcClientConfig::new(
            endpoint(),
        ));
        c.offer(b"v=0\r\n".to_vec()).unwrap();
        assert!(c.on_response(client::HttpResponse::new(status)).is_err());
        assert_eq!(*c.state(), client::State::Idle);
        assert!(c.offer(b"v=0\r\n".to_vec()).is_ok(), "{status}");
    }
}

/// Audit-22 #3006: an ICE-restart `PATCH` answered `200` without an `ETag`
/// cannot refresh the resource's tag, so the state machine says exactly that.
/// Bite test: dropping the `(OK, None)` arm turns it into a generic
/// `Http { status: 200 }`, hiding which header was missing.
#[test]
fn an_ice_restart_answer_without_an_etag_reports_the_missing_header() {
    let mut c = established_client();
    c.ice_restart(b"a=ice-ufrag:new\r\n".to_vec())
        .expect("an established session can restart ICE");
    let err = c
        .on_response(client::HttpResponse::new(StatusCode::OK))
        .expect_err("a 200 without an ETag cannot be applied");
    assert!(
        matches!(err, webrtc_runtime::Error::MissingHeader { header: "ETag" }),
        "{err:?}"
    );

    // The same answer carrying the new tag is the ICE-restart result.
    let mut c = established_client();
    c.ice_restart(b"a=ice-ufrag:new\r\n".to_vec()).unwrap();
    let event = c
        .on_response(
            client::HttpResponse::new(StatusCode::OK)
                .with_etag("e2")
                .unwrap()
                .with_body(b"a=ice-ufrag:peer\r\n".to_vec()),
        )
        .expect("a 200 with an ETag is the restart answer")
        .expect("an event is emitted");
    assert!(matches!(
        event,
        client::Event::IceRestart { new_etag, .. } if new_etag == "e2"
    ));
}

#[test]
fn late_ack_restart_does_not_resurrect_a_deleted_session() {
    let mut s = WhipSession::new(session_url());
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
    let mut s = WhipSession::new(session_url());
    s.on_post(b"v=0\r\n".to_vec()).unwrap();
    s.accept(b"v=0\r\n".to_vec(), "e1".into());
    s.on_delete().unwrap();
    let resp = s.accept(b"v=0\r\n".to_vec(), "e1".into());
    assert_eq!(resp.status, StatusCode::NOT_FOUND);
    assert_eq!(*s.state(), server::State::Closed);
}

#[test]
fn ack_answer_outside_a_counter_offer_is_refused() {
    let mut s = WhepSession::new(session_url());
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

/// #2859: `valid_until` was interpolated between `Content-Type` parameter
/// quotes with no validation, so a quote or CR/LF in it forged the rest of the
/// header block.
#[test]
fn counter_offer_rejects_a_valid_until_that_cannot_be_a_parameter() {
    let mut s = WhepSession::new(session_url());
    s.on_post(b"v=0\r\n".to_vec()).unwrap();
    for bad in [
        "2026-08-01T00:00:00Z\"",
        "2026-08-01T00:00:00Z\r\nX-Injected: 1",
        "2026-08-01T00:00:00Z>",
    ] {
        let resp = s.counter_offer(b"v=0\r\n".to_vec(), Some(bad.into()));
        assert_eq!(resp.status, StatusCode::INTERNAL_SERVER_ERROR, "{bad:?}");
        assert_eq!(
            *s.state(),
            server::State::AwaitingOffer,
            "state changes only on a sendable value: {bad:?}"
        );
    }

    let resp = s.counter_offer(b"v=0\r\n".to_vec(), Some("2026-08-01T00:00:00Z".into()));
    assert_eq!(resp.status, StatusCode::NOT_ACCEPTABLE);
    assert!(matches!(s.state(), server::State::CounterOffered { .. }));
}

#[test]
fn ack_answer_validates_the_etag_and_sends_it() {
    let mut s = WhepSession::new(session_url());
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
