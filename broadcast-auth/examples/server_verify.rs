//! Server-side challenge + verify (`broadcast_auth::Verifier`) — the origin
//! half of the RFC 7235 handshake: issue a `WWW-Authenticate` challenge, then
//! accept a correct `Authorization` response and reject a wrong one. Covers
//! Digest in depth (the scheme with the most moving parts — nonce/`nc`/`HA1`/
//! `HA2`), plus Basic, Bearer, and the reverse-proxy `Forwarded` scheme.
//!
//! Self-contained and non-blocking: no socket, no server actually run — just
//! the `Verifier` API a real origin (e.g. multimux's shared output-auth
//! middleware) would call.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example server_verify -p broadcast-auth
//! ```

use broadcast_auth::{AuthResult, Credentials, RequestContext, Verifier, respond};

const REALM: &str = "cameras";

fn main() {
    digest();
    basic();
    bearer();
    forwarded();
}

/// Digest (RFC 7616): the fullest round trip — challenge, correct response
/// accepted, wrong password rejected.
fn digest() {
    let verifier = Verifier::new(
        Credentials::Digest {
            username: "admin".into(),
            password: "hunter2".into(),
        },
        REALM,
    );
    let challenge = verifier.challenge();
    println!("[digest] challenge: {challenge}");

    let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/live");

    // Correct credential: respond() answers the challenge, verify() accepts.
    let correct = respond(&challenge, &ctx, Credentials::new("admin", "hunter2"))
        .expect("respond computes an Authorization value");
    let outcome = verify(&verifier, &correct, &ctx);
    println!("[digest] correct password  -> {outcome:?}");
    assert_eq!(outcome, AuthResult::Ok);

    // Wrong credential: same challenge, wrong password -> rejected.
    let wrong = respond(&challenge, &ctx, Credentials::new("admin", "WRONG"))
        .expect("respond computes an Authorization value even for a wrong password");
    let outcome = verify(&verifier, &wrong, &ctx);
    println!("[digest] wrong password    -> {outcome:?}");
    assert_eq!(outcome, AuthResult::Unauthorized);
}

/// Basic (RFC 7617): same accept/reject shape, briefly.
fn basic() {
    let verifier = Verifier::new(
        Credentials::Basic {
            username: "admin".into(),
            password: "hunter2".into(),
        },
        REALM,
    );
    let challenge = verifier.challenge();
    println!("[basic] challenge: {challenge}");

    let ctx = RequestContext::new("GET", "/stream/media.m3u8");
    let correct =
        respond(&challenge, &ctx, Credentials::new("admin", "hunter2")).expect("responds");
    assert_eq!(verify(&verifier, &correct, &ctx), AuthResult::Ok);
    println!("[basic] correct password   -> Ok");

    let wrong = respond(&challenge, &ctx, Credentials::new("admin", "WRONG")).expect("responds");
    assert_eq!(verify(&verifier, &wrong, &ctx), AuthResult::Unauthorized);
    println!("[basic] wrong password     -> Unauthorized");
}

/// Bearer (RFC 6750): no challenge round-trip needed, but still an
/// accept/reject pair — a wrong token must not verify.
fn bearer() {
    let verifier = Verifier::new(Credentials::bearer("right-token"), REALM);
    let challenge = verifier.challenge();
    println!("[bearer] challenge: {challenge}");

    let ctx = RequestContext::new("GET", "/stream/media.m3u8");
    let correct = respond(&challenge, &ctx, Credentials::bearer("right-token")).expect("responds");
    assert_eq!(verify(&verifier, &correct, &ctx), AuthResult::Ok);
    println!("[bearer] correct token     -> Ok");

    let wrong = respond(&challenge, &ctx, Credentials::bearer("wrong-token")).expect("responds");
    assert_eq!(verify(&verifier, &wrong, &ctx), AuthResult::Unauthorized);
    println!("[bearer] wrong token       -> Unauthorized");
}

/// Reverse-proxy forwarded-auth (`Verifier::forwarded`): no credential at
/// all — authenticated iff the proxy-set user header is present and
/// non-empty. See `Verifier::forwarded`'s doc for the trust assumption this
/// scheme relies on (only safe behind a proxy that strips client-supplied
/// copies of the header).
fn forwarded() {
    let verifier = Verifier::forwarded("X-Forwarded-User", Some("X-Forwarded-For".to_string()));
    println!("[forwarded] challenge: {}", verifier.challenge());

    let headers: &[(&str, &str)] = &[("X-Forwarded-User", "alice")];
    let ctx = RequestContext::new("GET", "/stream/media.m3u8").with_headers(headers);
    assert_eq!(verifier.verify(&ctx), AuthResult::Ok);
    println!("[forwarded] header present -> Ok");

    let ctx_no_header = RequestContext::new("GET", "/stream/media.m3u8");
    assert_eq!(verifier.verify(&ctx_no_header), AuthResult::Unauthorized);
    println!("[forwarded] header absent  -> Unauthorized");
}

/// Builds a request context carrying `authorization` as the `Authorization`
/// header, then verifies it against `verifier`.
fn verify(verifier: &Verifier, authorization: &str, ctx: &RequestContext<'_>) -> AuthResult {
    let headers: &[(&str, &str)] = &[("authorization", authorization)];
    let ctx_with_auth = RequestContext::new(ctx.method, ctx.uri).with_headers(headers);
    verifier.verify(&ctx_with_auth)
}
