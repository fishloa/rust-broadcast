//! WHEP (draft-ietf-wish-whep) state-machine round-trip tests.
//!
//! Exercises both direct-accept and counter-offer flows for the player
//! (viewer) and server sides — SDP offer/answer, counter-offer exchange,
//! trickle ICE, ICE restart, teardown, and the no-publisher 409 case.

use headers::HeaderMapExt;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use webrtc_runtime::Error;
use webrtc_runtime::whep::player::{self, WhepPlayer};
use webrtc_runtime::whep::server::{self, WhepSession};

const ENDPOINT: &str = "https://whep.example.com/watch";
const SESSION: &str = "https://whep.example.com/session/xyz789";
const SDP_OFFER: &[u8] = b"v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\n";
const SDP_ANSWER: &[u8] = b"v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n";
const SERVER_OFFER: &[u8] = b"v=0\r\no=server 0 0 IN IP4 10.0.0.1\r\n";
const ICE_FRAG: &[u8] = b"a=ice-ufrag:test\r\na=ice-pwd:test\r\n";

/// Headers of a PATCH request: `Content-Type` and an optional `If-Match`.
fn patch_headers(content_type: &str, if_match: Option<&str>) -> HeaderMap {
    let mut m = HeaderMap::new();
    m.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type).unwrap(),
    );
    if let Some(v) = if_match {
        m.insert(header::IF_MATCH, HeaderValue::from_str(v).unwrap());
    }
    m
}

// ---------------------------------------------------------------------------
// Player tests
// ---------------------------------------------------------------------------

#[test]
fn whep_player_direct_accept() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);
    assert_eq!(*player.state(), player::State::Idle);

    // 1. offer() — Idle -> OfferSent
    let req = player.offer(SDP_OFFER.to_vec()).unwrap();
    assert_eq!(req.method, http::Method::POST);
    assert_eq!(req.url, ENDPOINT);
    assert_eq!(req.content_type().unwrap().to_string(), "application/sdp");
    assert_eq!(req.body, SDP_OFFER);
    assert_eq!(*player.state(), player::State::OfferSent);

    // 2. on_response(201) — OfferSent -> Established, emits SdpAnswer
    let event = player
        .on_response(
            player::HttpResponse::new(StatusCode::CREATED)
                .with_content_type("application/sdp")
                .unwrap()
                .with_location(SESSION)
                .unwrap()
                .with_etag("e1")
                .unwrap()
                .with_body(SDP_ANSWER.to_vec()),
        )
        .unwrap();
    assert!(matches!(event, Some(player::Event::SdpAnswer(ref a)) if a == SDP_ANSWER));
    assert!(matches!(
        player.state(),
        player::State::Established { session_url, etag }
            if session_url == SESSION && etag.as_deref() == Some("e1")
    ));
}

#[test]
fn whep_player_counter_offer_flow() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);
    let _ = player.offer(SDP_OFFER.to_vec()).unwrap();

    // 1. on_response(406) — OfferSent -> CounterOffered, emits CounterOffer
    let event = player
        .on_response(
            player::HttpResponse::new(StatusCode::NOT_ACCEPTABLE)
                .with_content_type("application/sdp")
                .unwrap()
                .with_location(SESSION)
                .unwrap()
                .with_body(SERVER_OFFER.to_vec()),
        )
        .unwrap();
    match event {
        Some(player::Event::CounterOffer(ref body)) => {
            assert_eq!(body, SERVER_OFFER);
        }
        other => panic!("expected CounterOffer event, got {other:?}"),
    }
    assert!(matches!(
        player.state(),
        player::State::CounterOffered { session_url } if session_url == SESSION
    ));

    // 2. answer_counter_offer() — sends PATCH with application/sdp
    let req = player.answer_counter_offer(SDP_ANSWER.to_vec()).unwrap();
    assert_eq!(req.method, http::Method::PATCH);
    assert_eq!(req.url, SESSION);
    assert_eq!(req.content_type().unwrap().to_string(), "application/sdp");
    assert_eq!(req.body, SDP_ANSWER);

    // 3. on_response(204) — CounterOffered -> Established
    let event = player
        .on_response(
            player::HttpResponse::new(StatusCode::NO_CONTENT)
                .with_etag("e-after-counter")
                .unwrap(),
        )
        .unwrap();
    assert!(event.is_none());
    assert!(matches!(
        player.state(),
        player::State::Established { session_url, etag }
            if session_url == SESSION && etag.as_deref() == Some("e-after-counter")
    ));
}

#[test]
fn whep_player_no_publisher() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);
    let _ = player.offer(SDP_OFFER.to_vec()).unwrap();

    // 409 Conflict -> NoPublisher error
    let err = player.on_response(player::HttpResponse::new(StatusCode::CONFLICT));
    assert!(matches!(err.unwrap_err(), Error::NoPublisher));
}

#[test]
fn whep_player_trickle_ice_and_terminate() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);
    let _ = player.offer(SDP_OFFER.to_vec()).unwrap();
    let _ = player
        .on_response(
            player::HttpResponse::new(StatusCode::CREATED)
                .with_content_type("application/sdp")
                .unwrap()
                .with_location(SESSION)
                .unwrap()
                .with_etag("e1")
                .unwrap()
                .with_body(SDP_ANSWER.to_vec()),
        )
        .unwrap();

    // trickle_ice() — PATCH with If-Match
    let req = player.trickle_ice(ICE_FRAG.to_vec()).unwrap();
    assert_eq!(req.method, http::Method::PATCH);
    assert_eq!(req.url, SESSION);
    assert_eq!(
        req.content_type().unwrap().to_string(),
        "application/trickle-ice-sdpfrag"
    );
    let if_match = req.headers.get("If-Match").map(|v| v.to_str().unwrap());
    assert_eq!(if_match, Some("\"e1\""));

    // terminate() — DELETE
    let req = player.terminate().unwrap();
    assert_eq!(req.method, http::Method::DELETE);
    assert_eq!(req.url, SESSION);

    // 200 with no ETag -> Terminated
    let event = player
        .on_response(player::HttpResponse::new(StatusCode::OK))
        .unwrap();
    assert!(matches!(event, Some(player::Event::Terminated)));
    assert_eq!(*player.state(), player::State::Closed);
}

#[test]
fn whep_player_ice_restart() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);
    let _ = player.offer(SDP_OFFER.to_vec()).unwrap();
    let _ = player
        .on_response(
            player::HttpResponse::new(StatusCode::CREATED)
                .with_content_type("application/sdp")
                .unwrap()
                .with_location(SESSION)
                .unwrap()
                .with_etag("e1")
                .unwrap()
                .with_body(SDP_ANSWER.to_vec()),
        )
        .unwrap();

    // ICE restart — If-Match: "*"
    let req = player.ice_restart(ICE_FRAG.to_vec()).unwrap();
    let if_match = req.headers.get("If-Match").map(|v| v.to_str().unwrap());
    assert_eq!(if_match, Some("*"));

    // 200 with new ETag
    let event = player
        .on_response(
            player::HttpResponse::new(StatusCode::OK)
                .with_content_type("application/trickle-ice-sdpfrag")
                .unwrap()
                .with_etag("e2")
                .unwrap()
                .with_body(b"a=ice-ufrag:srv\r\n".to_vec()),
        )
        .unwrap();
    match event {
        Some(player::Event::IceRestart { new_etag, .. }) => {
            assert_eq!(new_etag, "e2");
        }
        other => panic!("expected IceRestart, got {other:?}"),
    }
    assert!(matches!(
        player.state(),
        player::State::Established { etag: Some(e), .. } if e == "e2"
    ));
}

// Regression (audit run-09 W21, mirroring the WHIP client fix): a
// trickle-ICE PATCH answered `200` with an `ETag` must not be misread as an
// ICE-restart answer.
#[test]
fn whep_player_trickle_ack_with_200_and_etag_is_not_misread_as_ice_restart() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);
    let _ = player.offer(SDP_OFFER.to_vec()).unwrap();
    let _ = player
        .on_response(
            player::HttpResponse::new(StatusCode::CREATED)
                .with_content_type("application/sdp")
                .unwrap()
                .with_location(SESSION)
                .unwrap()
                .with_etag("e1")
                .unwrap()
                .with_body(SDP_ANSWER.to_vec()),
        )
        .unwrap();

    let _ = player.trickle_ice(ICE_FRAG.to_vec()).unwrap();
    let event = player
        .on_response(
            player::HttpResponse::new(StatusCode::OK)
                .with_etag("e2")
                .unwrap(),
        )
        .unwrap();
    assert!(
        event.is_none(),
        "a trickle ack must never produce an event, got {event:?}"
    );
    assert!(matches!(
        player.state(),
        player::State::Established { etag: Some(e), .. } if e == "e2"
    ));
}

// Regression (audit run-09 W21): a `DELETE` answered `204` must still
// transition to `Closed` and emit `Terminated`.
#[test]
fn whep_player_terminate_ack_with_204_transitions_to_closed() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);
    let _ = player.offer(SDP_OFFER.to_vec()).unwrap();
    let _ = player
        .on_response(
            player::HttpResponse::new(StatusCode::CREATED)
                .with_content_type("application/sdp")
                .unwrap()
                .with_location(SESSION)
                .unwrap()
                .with_etag("e1")
                .unwrap()
                .with_body(SDP_ANSWER.to_vec()),
        )
        .unwrap();

    let _ = player.terminate().unwrap();
    let event = player
        .on_response(player::HttpResponse::new(StatusCode::NO_CONTENT))
        .unwrap();
    assert!(matches!(event, Some(player::Event::Terminated)));
    assert_eq!(*player.state(), player::State::Closed);
}

#[test]
fn whep_player_wrong_state_errors() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);

    // answer_counter_offer in Idle -> error
    assert!(player.answer_counter_offer(SDP_ANSWER.to_vec()).is_err());

    // trickle_ice in Idle -> error
    assert!(player.trickle_ice(ICE_FRAG.to_vec()).is_err());

    // terminate in Idle -> error
    assert!(player.terminate().is_err());

    // offer twice -> error
    let _ = player.offer(SDP_OFFER.to_vec()).unwrap();
    assert!(player.offer(SDP_OFFER.to_vec()).is_err());
}

#[test]
fn whep_player_bearer_auth() {
    let token = "viewer-token";
    let mut player = WhepPlayer::new(ENDPOINT.into(), Some(token.into()));

    let req = player.offer(SDP_OFFER.to_vec()).unwrap();
    let auth = req
        .headers
        .get("Authorization")
        .map(|v| v.to_str().unwrap());
    assert_eq!(auth, Some("Bearer viewer-token"));
}

#[test]
fn whep_player_missing_location_on_201() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);
    let _ = player.offer(SDP_OFFER.to_vec()).unwrap();

    let err = player.on_response(
        player::HttpResponse::new(StatusCode::CREATED)
            .with_content_type("application/sdp")
            .unwrap()
            .with_etag("e")
            .unwrap()
            .with_body(SDP_ANSWER.to_vec()),
    );
    assert!(matches!(
        err.unwrap_err(),
        Error::MissingHeader { header: "Location" }
    ));
}

#[test]
fn whep_player_missing_location_on_406() {
    let mut player = WhepPlayer::new(ENDPOINT.into(), None);
    let _ = player.offer(SDP_OFFER.to_vec()).unwrap();

    let err = player.on_response(
        player::HttpResponse::new(StatusCode::NOT_ACCEPTABLE)
            .with_content_type("application/sdp")
            .unwrap()
            .with_body(SERVER_OFFER.to_vec()),
    );
    assert!(matches!(
        err.unwrap_err(),
        Error::MissingHeader { header: "Location" }
    ));
}

// ---------------------------------------------------------------------------
// Server tests
// ---------------------------------------------------------------------------

#[test]
fn whep_server_direct_accept() {
    let mut session = WhepSession::new(SESSION.into());
    assert_eq!(*session.state(), server::State::AwaitingOffer);

    // 1. on_post() — emits SdpOffer
    let event = session.on_post(SDP_OFFER.to_vec()).unwrap();
    assert!(matches!(event, server::Event::SdpOffer(ref o) if o == SDP_OFFER));

    // 2. accept() — transitions to Established, returns 201
    let resp = session.accept(SDP_ANSWER.to_vec(), "e1".into());
    assert_eq!(resp.status, StatusCode::CREATED);
    assert_eq!(
        resp.headers
            .typed_get::<headers::ContentType>()
            .unwrap()
            .to_string(),
        "application/sdp"
    );
    assert_eq!(resp.body, SDP_ANSWER);
    let location = resp.headers.get("Location");
    assert_eq!(location.unwrap(), SESSION);
    assert!(matches!(
        session.state(),
        server::State::Established { etag } if etag == "e1"
    ));
}

#[test]
fn whep_server_counter_offer() {
    let mut session = WhepSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();

    // 1. counter_offer() — transitions to CounterOffered, returns 406
    let resp = session.counter_offer(SERVER_OFFER.to_vec(), None);
    assert_eq!(resp.status, StatusCode::NOT_ACCEPTABLE);
    assert_eq!(resp.body, SERVER_OFFER);
    let location = resp.headers.get("Location");
    assert_eq!(location.unwrap(), SESSION);
    assert!(matches!(
        session.state(),
        server::State::CounterOffered { .. }
    ));

    // 2. on_patch(application/sdp) — emits SdpAnswer
    let event = session
        .on_patch(SDP_ANSWER.to_vec(), &patch_headers("application/sdp", None))
        .unwrap();
    assert!(matches!(event, server::Event::SdpAnswer(ref a) if a == SDP_ANSWER));

    // 3. ack_answer() — transitions to Established, returns 204
    let resp = session.ack_answer("e-final".into());
    assert_eq!(resp.status, StatusCode::NO_CONTENT);
    assert!(matches!(
        session.state(),
        server::State::Established { etag } if etag == "e-final"
    ));
}

#[test]
fn whep_server_counter_offer_with_valid_until() {
    let mut session = WhepSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();

    let resp = session.counter_offer(SERVER_OFFER.to_vec(), Some("2026-08-01T00:00:00Z".into()));
    assert_eq!(resp.status, StatusCode::NOT_ACCEPTABLE);
    // Content-Type should include valid-until parameter
    let ct = resp
        .headers
        .get("Content-Type")
        .map(|v| v.to_str().unwrap());
    assert!(ct.unwrap().contains("valid-until=\"2026-08-01T00:00:00Z\""));
}

#[test]
fn whep_server_no_publisher_response() {
    // no_publisher is a static method — no session needed
    let resp = WhepSession::no_publisher(Some(std::time::Duration::from_secs(30)));
    assert_eq!(resp.status, StatusCode::CONFLICT);
    let retry = resp.headers.get("Retry-After").map(|v| v.to_str().unwrap());
    assert_eq!(retry, Some("30"));
    assert!(resp.body.is_empty());

    // Without Retry-After
    let resp = WhepSession::no_publisher(None);
    assert_eq!(resp.status, StatusCode::CONFLICT);
    assert!(resp.headers.is_empty());
}

#[test]
fn whep_server_trickle_ice() {
    let mut session = WhepSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.accept(SDP_ANSWER.to_vec(), "e1".into());

    // Trickle ICE with matching ETag
    let event = session
        .on_patch(
            ICE_FRAG.to_vec(),
            &patch_headers("application/trickle-ice-sdpfrag", Some("\"e1\"")),
        )
        .unwrap();
    match event {
        server::Event::TrickleIce {
            sdp_fragment,
            if_match,
        } => {
            assert_eq!(sdp_fragment, ICE_FRAG);
            assert_eq!(if_match.as_deref(), Some("e1"));
        }
        other => panic!("expected TrickleIce, got {other:?}"),
    }

    let resp = session.ack_trickle();
    assert_eq!(resp.status, StatusCode::NO_CONTENT);
}

#[test]
fn whep_server_ice_restart() {
    let mut session = WhepSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.accept(SDP_ANSWER.to_vec(), "e1".into());

    // ICE restart — If-Match: "*"
    let event = session
        .on_patch(
            ICE_FRAG.to_vec(),
            &patch_headers("application/trickle-ice-sdpfrag", Some("*")),
        )
        .unwrap();
    assert!(matches!(event, server::Event::IceRestart { .. }));

    let resp = session.ack_restart(b"a=ice-ufrag:srv\r\n".to_vec(), "e2".into());
    assert_eq!(resp.status, StatusCode::OK);
    assert!(matches!(
        session.state(),
        server::State::Established { etag } if etag == "e2"
    ));
}

#[test]
fn whep_server_etag_mismatch() {
    let mut session = WhepSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.accept(SDP_ANSWER.to_vec(), "correct".into());

    let err = session.on_patch(
        ICE_FRAG.to_vec(),
        &patch_headers("application/trickle-ice-sdpfrag", Some("\"wrong\"")),
    );
    assert!(matches!(
        err.unwrap_err(),
        Error::ETagMismatch { expected, got } if expected == "correct" && got == "\"wrong\""
    ));
}

#[test]
fn whep_server_wrong_content_type_in_established() {
    let mut session = WhepSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.accept(SDP_ANSWER.to_vec(), "e".into());

    // Wrong content-type PATCH in Established -> error
    let err = session.on_patch(ICE_FRAG.to_vec(), &patch_headers("text/plain", None));
    assert!(matches!(err.unwrap_err(), Error::InvalidSdpFragment { .. }));
}

// Regression (audit run-09 W21): a counter-offer answer sent as
// `application/sdp; charset=utf-8` (a legal media type with a parameter,
// RFC 9110 §8.3.1) must be accepted, not rejected by a strict `==` compare
// against the bare `application/sdp` constant.
#[test]
fn whep_server_accepts_content_type_with_a_parameter() {
    let mut session = WhepSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.counter_offer(SERVER_OFFER.to_vec(), None);

    let event = session
        .on_patch(
            SDP_ANSWER.to_vec(),
            &patch_headers("application/sdp; charset=utf-8", None),
        )
        .unwrap();
    assert!(matches!(event, server::Event::SdpAnswer(ref a) if a == SDP_ANSWER));
}

#[test]
fn whep_server_delete_from_counter_offered() {
    let mut session = WhepSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.counter_offer(SERVER_OFFER.to_vec(), None);

    // DELETE from CounterOffered should work
    let event = session.on_delete().unwrap();
    assert!(matches!(event, server::Event::Terminated));
    assert_eq!(*session.state(), server::State::Closed);
}

#[test]
fn whep_server_wrong_state_errors() {
    let mut session = WhepSession::new(SESSION.into());

    // PATCH before any offer -> error
    let err = session.on_patch(SDP_ANSWER.to_vec(), &patch_headers("application/sdp", None));
    assert!(matches!(err.unwrap_err(), Error::WrongState { .. }));

    // DELETE before any offer -> error
    assert!(session.on_delete().is_err());

    // Post twice -> error
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.accept(SDP_ANSWER.to_vec(), "e".into());
    assert!(session.on_post(SDP_OFFER.to_vec()).is_err());
}
