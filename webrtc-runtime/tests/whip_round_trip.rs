//! WHIP (RFC 9725) state-machine round-trip tests.
//!
//! Exercises the full client and server lifecycle — SDP offer/answer,
//! trickle ICE, ICE restart, teardown — and verifies state transitions
//! and wrong-state rejections at each step.

use webrtc_runtime::Error;
use webrtc_runtime::whip::client::{self, WhipClient};
use webrtc_runtime::whip::server::{self, WhipSession};

const ENDPOINT: &str = "https://whip.example.com/pub";
const SESSION: &str = "https://whip.example.com/session/abc123";
const SDP_OFFER: &[u8] = b"v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\n";
const SDP_ANSWER: &[u8] = b"v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n";
const ICE_FRAG: &[u8] = b"a=ice-ufrag:test\r\na=ice-pwd:test\r\n";

// ---------------------------------------------------------------------------
// Client tests
// ---------------------------------------------------------------------------

#[test]
fn whip_client_full_lifecycle() {
    let mut client = WhipClient::new(ENDPOINT.into(), None);
    assert_eq!(*client.state(), client::State::Idle);

    // 1. offer() — transitions Idle -> OfferSent
    let req = client.offer(SDP_OFFER.to_vec()).unwrap();
    assert_eq!(req.method, client::Method::Post);
    assert_eq!(req.url, ENDPOINT);
    assert_eq!(req.content_type, Some("application/sdp"));
    assert_eq!(req.body, SDP_OFFER);
    assert_eq!(*client.state(), client::State::OfferSent);

    // 2. on_response(201) — transitions OfferSent -> Established
    let event = client
        .on_response(client::HttpResponse {
            status: 201,
            content_type: Some("application/sdp".into()),
            location: Some(SESSION.into()),
            etag: Some("etag1".into()),
            body: SDP_ANSWER.to_vec(),
        })
        .unwrap();
    assert!(matches!(event, Some(client::Event::SdpAnswer(ref a)) if a == SDP_ANSWER));
    assert!(matches!(
        client.state(),
        client::State::Established { session_url, etag }
            if session_url == SESSION && etag.as_deref() == Some("etag1")
    ));

    // 3. flush_candidates() — stays Established, sends PATCH
    let req = client.flush_candidates(ICE_FRAG.to_vec()).unwrap();
    assert_eq!(req.method, client::Method::Patch);
    assert_eq!(req.url, SESSION);
    assert_eq!(req.content_type, Some("application/trickle-ice-sdpfrag"));
    // If-Match header should carry the ETag
    let if_match = req.headers.iter().find(|(k, _)| k == "If-Match");
    assert!(if_match.is_some());
    assert_eq!(if_match.unwrap().1, "\"etag1\"");

    // 4. on_response(204) — trickle acknowledged, no event
    let event = client
        .on_response(client::HttpResponse {
            status: 204,
            content_type: None,
            location: None,
            etag: None,
            body: Vec::new(),
        })
        .unwrap();
    assert!(event.is_none());

    // 5. terminate() — sends DELETE
    let req = client.terminate().unwrap();
    assert_eq!(req.method, client::Method::Delete);
    assert_eq!(req.url, SESSION);
    assert!(req.body.is_empty());

    // 6. on_response(200, no ETag) — transitions Established -> Closed
    let event = client
        .on_response(client::HttpResponse {
            status: 200,
            content_type: None,
            location: None,
            etag: None,
            body: Vec::new(),
        })
        .unwrap();
    assert!(matches!(event, Some(client::Event::Terminated)));
    assert_eq!(*client.state(), client::State::Closed);
}

#[test]
fn whip_client_wrong_state_errors() {
    let mut client = WhipClient::new(ENDPOINT.into(), None);

    // flush_candidates in Idle -> error
    let err = client.flush_candidates(ICE_FRAG.to_vec());
    assert!(err.is_err());
    assert!(
        matches!(err.unwrap_err(), Error::WrongState { operation, .. } if operation.contains("established"))
    );

    // terminate in Idle -> error
    assert!(client.terminate().is_err());

    // ice_restart in Idle -> error
    assert!(client.ice_restart(ICE_FRAG.to_vec()).is_err());

    // on_response in Idle -> error
    let err = client.on_response(client::HttpResponse {
        status: 200,
        content_type: None,
        location: None,
        etag: None,
        body: Vec::new(),
    });
    assert!(matches!(err.unwrap_err(), Error::WrongState { .. }));

    // Advance to OfferSent
    let _ = client.offer(SDP_OFFER.to_vec()).unwrap();
    assert_eq!(*client.state(), client::State::OfferSent);

    // offer() again in OfferSent -> error
    let err = client.offer(SDP_OFFER.to_vec());
    assert!(matches!(
        err.unwrap_err(),
        Error::WrongState { operation, .. } if operation == "offer"
    ));

    // flush_candidates in OfferSent -> error
    assert!(client.flush_candidates(ICE_FRAG.to_vec()).is_err());

    // Advance to Established
    let _ = client
        .on_response(client::HttpResponse {
            status: 201,
            content_type: Some("application/sdp".into()),
            location: Some(SESSION.into()),
            etag: Some("e1".into()),
            body: SDP_ANSWER.to_vec(),
        })
        .unwrap();

    // Move to Closed
    let _ = client.terminate().unwrap();
    let _ = client
        .on_response(client::HttpResponse {
            status: 200,
            content_type: None,
            location: None,
            etag: None,
            body: Vec::new(),
        })
        .unwrap();
    assert_eq!(*client.state(), client::State::Closed);

    // Everything fails in Closed
    assert!(client.offer(SDP_OFFER.to_vec()).is_err());
    assert!(client.flush_candidates(ICE_FRAG.to_vec()).is_err());
    assert!(client.terminate().is_err());
}

#[test]
fn whip_client_ice_restart() {
    let mut client = WhipClient::new(ENDPOINT.into(), None);
    let _ = client.offer(SDP_OFFER.to_vec()).unwrap();
    let _ = client
        .on_response(client::HttpResponse {
            status: 201,
            content_type: Some("application/sdp".into()),
            location: Some(SESSION.into()),
            etag: Some("etag-v1".into()),
            body: SDP_ANSWER.to_vec(),
        })
        .unwrap();

    // ICE restart — sends PATCH with If-Match: "*"
    let restart_frag = b"a=ice-ufrag:new\r\na=ice-pwd:new\r\n";
    let req = client.ice_restart(restart_frag.to_vec()).unwrap();
    assert_eq!(req.method, client::Method::Patch);
    assert_eq!(req.url, SESSION);
    let if_match = req
        .headers
        .iter()
        .find(|(k, _)| k == "If-Match")
        .map(|(_, v)| v.as_str());
    assert_eq!(if_match, Some("*"));

    // Server responds 200 with new ETag + server's new fragment
    let server_frag = b"a=ice-ufrag:srv\r\na=ice-pwd:srv\r\n";
    let event = client
        .on_response(client::HttpResponse {
            status: 200,
            content_type: Some("application/trickle-ice-sdpfrag".into()),
            location: None,
            etag: Some("etag-v2".into()),
            body: server_frag.to_vec(),
        })
        .unwrap();

    match event {
        Some(client::Event::IceRestart {
            sdp_fragment,
            new_etag,
        }) => {
            assert_eq!(sdp_fragment, server_frag);
            assert_eq!(new_etag, "etag-v2");
        }
        other => panic!("expected IceRestart event, got {other:?}"),
    }

    // Verify ETag was updated in state
    assert!(matches!(
        client.state(),
        client::State::Established { etag: Some(e), .. } if e == "etag-v2"
    ));
}

#[test]
fn whip_client_bearer_auth() {
    let token = "my-secret-token";
    let mut client = WhipClient::new(ENDPOINT.into(), Some(token.into()));

    // POST offer — should have Authorization header
    let req = client.offer(SDP_OFFER.to_vec()).unwrap();
    let auth = req
        .headers
        .iter()
        .find(|(k, _)| k == "Authorization")
        .map(|(_, v)| v.as_str());
    assert_eq!(auth, Some("Bearer my-secret-token"));

    // Establish session
    let _ = client
        .on_response(client::HttpResponse {
            status: 201,
            content_type: Some("application/sdp".into()),
            location: Some(SESSION.into()),
            etag: Some("e".into()),
            body: SDP_ANSWER.to_vec(),
        })
        .unwrap();

    // PATCH — should also have Authorization header
    let req = client.flush_candidates(ICE_FRAG.to_vec()).unwrap();
    let auth = req
        .headers
        .iter()
        .find(|(k, _)| k == "Authorization")
        .map(|(_, v)| v.as_str());
    assert_eq!(auth, Some("Bearer my-secret-token"));

    // DELETE — should also have Authorization header
    let req = client.terminate().unwrap();
    let auth = req
        .headers
        .iter()
        .find(|(k, _)| k == "Authorization")
        .map(|(_, v)| v.as_str());
    assert_eq!(auth, Some("Bearer my-secret-token"));
}

#[test]
fn whip_client_missing_location_on_201() {
    let mut client = WhipClient::new(ENDPOINT.into(), None);
    let _ = client.offer(SDP_OFFER.to_vec()).unwrap();

    let err = client.on_response(client::HttpResponse {
        status: 201,
        content_type: Some("application/sdp".into()),
        location: None,
        etag: Some("e".into()),
        body: SDP_ANSWER.to_vec(),
    });
    assert!(matches!(
        err.unwrap_err(),
        Error::MissingHeader { header: "Location" }
    ));
}

#[test]
fn whip_client_http_error_propagated() {
    let mut client = WhipClient::new(ENDPOINT.into(), None);
    let _ = client.offer(SDP_OFFER.to_vec()).unwrap();

    let err = client.on_response(client::HttpResponse {
        status: 503,
        content_type: None,
        location: None,
        etag: None,
        body: Vec::new(),
    });
    assert!(matches!(err.unwrap_err(), Error::Http { status: 503 }));
}

// ---------------------------------------------------------------------------
// Server tests
// ---------------------------------------------------------------------------

#[test]
fn whip_server_full_lifecycle() {
    let mut session = WhipSession::new(SESSION.into());
    assert_eq!(*session.state(), server::State::AwaitingOffer);
    assert_eq!(session.session_url(), SESSION);

    // 1. on_post() — emits SdpOffer event
    let event = session.on_post(SDP_OFFER.to_vec()).unwrap();
    assert!(matches!(event, server::Event::SdpOffer(ref o) if o == SDP_OFFER));

    // 2. accept() — transitions to Established, returns 201
    let resp = session.accept(SDP_ANSWER.to_vec(), "etag1".into());
    assert_eq!(resp.status, 201);
    assert_eq!(resp.content_type, Some("application/sdp"));
    assert_eq!(resp.body, SDP_ANSWER);
    // Location header
    let location = resp.headers.iter().find(|(k, _)| k == "Location");
    assert_eq!(location.unwrap().1, SESSION);
    // ETag header
    let etag = resp.headers.iter().find(|(k, _)| k == "ETag");
    assert_eq!(etag.unwrap().1, "\"etag1\"");
    assert!(matches!(
        session.state(),
        server::State::Established { etag } if etag == "etag1"
    ));

    // 3. on_patch() — trickle ICE (matching ETag)
    let event = session
        .on_patch(ICE_FRAG.to_vec(), Some("\"etag1\""))
        .unwrap();
    assert!(matches!(event, server::Event::TrickleIce { .. }));

    // 4. ack_trickle() — returns 204
    let resp = session.ack_trickle();
    assert_eq!(resp.status, 204);
    assert!(resp.body.is_empty());

    // 5. on_delete() — transitions to Closed
    let event = session.on_delete().unwrap();
    assert!(matches!(event, server::Event::Terminated));
    assert_eq!(*session.state(), server::State::Closed);

    // 6. ack_delete() — returns 200
    let resp = session.ack_delete();
    assert_eq!(resp.status, 200);
}

#[test]
fn whip_server_ice_restart() {
    let mut session = WhipSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.accept(SDP_ANSWER.to_vec(), "e1".into());

    // ICE restart — If-Match: "*"
    let restart_frag = b"a=ice-ufrag:new\r\n";
    let event = session.on_patch(restart_frag.to_vec(), Some("*")).unwrap();
    assert!(matches!(event, server::Event::IceRestart { .. }));

    // ack_restart — updates ETag, returns 200
    let server_frag = b"a=ice-ufrag:srv\r\n";
    let resp = session.ack_restart(server_frag.to_vec(), "e2".into());
    assert_eq!(resp.status, 200);
    assert_eq!(resp.content_type, Some("application/trickle-ice-sdpfrag"));
    assert_eq!(resp.body, server_frag);
    let etag = resp.headers.iter().find(|(k, _)| k == "ETag");
    assert_eq!(etag.unwrap().1, "\"e2\"");
    assert!(matches!(
        session.state(),
        server::State::Established { etag } if etag == "e2"
    ));
}

#[test]
fn whip_server_etag_mismatch() {
    let mut session = WhipSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.accept(SDP_ANSWER.to_vec(), "correct".into());

    let err = session.on_patch(ICE_FRAG.to_vec(), Some("\"wrong\""));
    assert!(matches!(
        err.unwrap_err(),
        Error::ETagMismatch { expected, got } if expected == "correct" && got == "wrong"
    ));
}

#[test]
fn whip_server_wrong_state_errors() {
    let mut session = WhipSession::new(SESSION.into());

    // PATCH before offer accepted -> error
    let err = session.on_patch(ICE_FRAG.to_vec(), None);
    assert!(matches!(err.unwrap_err(), Error::WrongState { .. }));

    // DELETE before established -> error
    let err = session.on_delete();
    assert!(matches!(err.unwrap_err(), Error::WrongState { .. }));

    // Accept and then try on_post again -> error
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.accept(SDP_ANSWER.to_vec(), "e".into());
    let err = session.on_post(SDP_OFFER.to_vec());
    assert!(matches!(err.unwrap_err(), Error::WrongState { .. }));

    // Close and try everything -> errors
    let _ = session.on_delete().unwrap();
    assert!(session.on_post(SDP_OFFER.to_vec()).is_err());
    assert!(session.on_patch(ICE_FRAG.to_vec(), None).is_err());
    assert!(session.on_delete().is_err());
}

#[test]
fn whip_server_trickle_no_if_match() {
    let mut session = WhipSession::new(SESSION.into());
    let _ = session.on_post(SDP_OFFER.to_vec()).unwrap();
    let _ = session.accept(SDP_ANSWER.to_vec(), "e".into());

    // PATCH without If-Match is valid for trickle ICE
    let event = session.on_patch(ICE_FRAG.to_vec(), None).unwrap();
    match event {
        server::Event::TrickleIce {
            sdp_fragment,
            if_match,
        } => {
            assert_eq!(sdp_fragment, ICE_FRAG);
            assert!(if_match.is_none());
        }
        other => panic!("expected TrickleIce, got {other:?}"),
    }
}
