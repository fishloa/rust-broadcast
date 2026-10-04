#![cfg(all(feature = "whep", feature = "test-hooks"))]
//! SP2.1/SP2.4: the WHEP endpoint on axum, sharing the origin's output-auth
//! middleware — a 401 must still carry CORS so a cross-origin browser can
//! see the challenge.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n";

/// BITING: pre-fix, `check_output_auth` hand-builds the 401 with only
/// `WWW-Authenticate` — no `Access-Control-Allow-Origin` — so this FAILS on
/// old code. The new order (CORS outside the auth middleware) must produce
/// both headers.
#[tokio::test]
async fn whep_post_without_credentials_is_401_with_challenge_and_cors() {
    let (app, _token) =
        multimux::output::whep::serve_whep_for_test(Some(basic_verifier("user", "pass"))).await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/whep")
                .header("content-type", "application/sdp")
                .header("origin", "http://player.example")
                .body(Body::from(OFFER))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(
        resp.headers().contains_key("www-authenticate"),
        "the Basic/Digest challenge must be present"
    );
    assert_eq!(
        resp.headers()["access-control-allow-origin"],
        "*",
        "CORS must survive the auth middleware (it is layered outside)"
    );
}

/// CHARACTERISATION (pins existing behaviour across the router move).
#[tokio::test]
async fn whep_post_with_credentials_against_a_live_trunk_is_201_with_location() {
    let (app, _token) = multimux::output::whep::serve_whep_for_test_with_trunk(None).await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/whep")
                .header("content-type", "application/sdp")
                .body(Body::from(OFFER))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert!(
        resp.headers().contains_key("location"),
        "Location must be present"
    );
    assert_eq!(
        resp.headers()["access-control-expose-headers"],
        "Location",
        "main exposed Location so a cross-origin browser can DELETE by it"
    );
    assert_eq!(resp.headers()["content-type"], "application/sdp");
}

/// CHARACTERISATION.
#[tokio::test]
async fn whep_options_preflight_is_204_with_cors() {
    let (app, _token) = multimux::output::whep::serve_whep_for_test(None).await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/whep")
                .header("origin", "http://player.example")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(
        resp.headers().contains_key("access-control-allow-origin"),
        "the preflight must carry CORS"
    );
}

/// CHARACTERISATION: RequestBodyLimitLayer rejects on Content-Length alone.
#[tokio::test]
async fn a_body_over_64_kib_is_rejected_413() {
    let (app, _token) = multimux::output::whep::serve_whep_for_test(None).await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/whep")
                .header("content-type", "application/sdp")
                .header("content-length", "131073")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

fn basic_verifier(username: &str, password: &str) -> std::sync::Arc<broadcast_auth::Verifier> {
    std::sync::Arc::new(broadcast_auth::Verifier::new(
        broadcast_auth::Credentials::Basic {
            username: username.to_string(),
            password: password.to_string(),
        },
        "whep-test",
    ))
}

/// I7: main answered every non-POST/non-OPTIONS method `405 Method Not
/// Allowed` (no PATCH/DELETE teardown in this cut), so the session routes must
/// NOT return a success without effect. PATCH and DELETE keep that parity
/// with an `Allow` header.
#[tokio::test]
async fn whep_patch_and_delete_are_405_with_allow() {
    let (app, _token) = multimux::output::whep::serve_whep_for_test(None).await;
    for method in ["PATCH", "DELETE"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/whep/session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} must be 405, not a stub 200/204"
        );
        assert!(
            resp.headers().contains_key("allow"),
            "{method} 405 must carry an Allow header"
        );
    }
}
