//! Client-side challenge->response (`broadcast_auth::respond` /
//! `broadcast_auth::Authenticator`) — given a server's `WWW-Authenticate`
//! challenge, compute the `Authorization` header value to answer it, for
//! Basic, Digest, and Bearer.
//!
//! Self-contained and non-blocking: no socket, no server — just the client
//! half of the handshake a real RTSP/HTTP client (e.g. `rtsp-runtime`,
//! multimux's HTTP input adapters) would send.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example client_respond -p broadcast-auth
//! ```

use broadcast_auth::{Authenticator, Credentials, RequestContext, respond};

fn main() {
    // --- Basic (RFC 7617): a one-shot respond(), no session state needed.
    let value = respond(
        "Basic realm=\"cameras\"",
        &RequestContext::new("GET", "/stream/media.m3u8"),
        Credentials::new("admin", "hunter2"),
    )
    .expect("Basic responds to any challenge shape");
    println!("[basic]  Authorization: {value}");
    assert!(value.starts_with("Basic "));

    // --- Digest (RFC 7616): parses the server's nonce/realm/qop out of the
    // challenge, then computes HA1/HA2/response. Demonstrated with an
    // Authenticator (not the one-shot respond()) since a real session reuses
    // it across requests so the nonce count (`nc`) advances correctly.
    let digest_challenge = "Digest realm=\"cameras\", \
        nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", qop=\"auth\", algorithm=MD5";
    let mut auth =
        Authenticator::from_challenge(digest_challenge, Credentials::new("admin", "hunter2"))
            .expect("challenge parses");
    let value = auth
        .authorization(&RequestContext::new(
            "DESCRIBE",
            "rtsp://camera.example.com/live",
        ))
        .expect("computes a Digest Authorization value");
    println!("[digest] Authorization: {value}");
    assert!(value.starts_with("Digest "));

    // A second request on the same Authenticator advances `nc` — the
    // computed value differs even though nothing else about the request
    // changed (RFC 7616 §3.3 requires a fresh `nc` per request).
    let second = auth
        .authorization(&RequestContext::new(
            "DESCRIBE",
            "rtsp://camera.example.com/live",
        ))
        .expect("computes a second Digest Authorization value");
    assert_ne!(value, second, "nc must advance across requests");
    println!("[digest] Authorization (2nd request, nc advanced): {second}");

    // --- Bearer (RFC 6750): no challenge round-trip needed at all — the
    // challenge value is ignored, the token is sent verbatim.
    let value = respond(
        "ignored — Bearer needs no challenge round-trip",
        &RequestContext::new("GET", "/stream/media.m3u8"),
        Credentials::bearer("mytoken123"),
    )
    .expect("Bearer always responds");
    println!("[bearer] Authorization: {value}");
    assert_eq!(value, "Bearer mytoken123");
}
