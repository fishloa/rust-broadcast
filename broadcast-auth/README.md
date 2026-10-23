# broadcast-auth

[![crates.io](https://img.shields.io/crates/v/broadcast-auth.svg)](https://crates.io/crates/broadcast-auth)
[![docs.rs](https://img.shields.io/docsrs/broadcast-auth)](https://docs.rs/broadcast-auth)

Shared multi-scheme HTTP and RTSP authentication: a client that answers `WWW-Authenticate` challenges, and a server that issues and verifies them (Basic, Digest, Bearer, reverse-proxy Forwarded, signed URLs).

## What it does / what it does not

**Does:** one `Credentials` model (`Basic`/`Digest` user and password, or a `Bearer` token); a client side (`Authenticator`, `respond`) that turns a server challenge into an `Authorization` value and keeps Digest's nonce count advancing across a session; a server side (`server::Verifier`) that renders the `WWW-Authenticate` challenge for a `401` and verifies an incoming `RequestContext` (method, URI, body, headers, peer address); and typed `WWW-Authenticate` challenge parsing (`challenge::parse_challenges`) including RFC 9728 `resource_metadata`. Secrets are wiped on drop and compared in constant time.

**Does not:** do any IO (sockets, header framing, TLS), store credentials or parse config, or implement OAuth flows. Digest offers `qop=auth` with MD5 only (the shape every camera and player in the workspace uses); `auth-int` and SHA-256 Digest are not offered by the verifier. Each client or server owns its transport: `rtsp-runtime`, `hls-runtime` and `multimux` call into this crate.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| RFC 7617 HTTP Basic | RFC 7617 (not vendored, unverified here) | Client response, server verify (constant-time) | Full | Challenge parsing delegated to `http-auth` |
| RFC 7616 HTTP Digest | RFC 7616 (not vendored, unverified here) | Client response (MD5, `qop=auth`, advancing `nc`), server verify incl. §3.4.4 `username*` and claimed-`uri` check | Partial | MD5 and `qop=auth` only. Server nonces are signed (HMAC-SHA256), expire after `DIGEST_NONCE_LIFETIME` (3600 s) and carry a 64-value anti-replay window per `(nonce, cnonce)`, capped at `DIGEST_NC_TRACK_CAP` |
| RFC 6750 Bearer token | RFC 6750 (not vendored, unverified here) | `Authorization: Bearer <token>` | Full | No challenge round-trip |
| RFC 7235 HTTP authentication framework | RFC 7235 (not vendored, unverified here) | `WWW-Authenticate` / `Authorization` header handling | Full | Via `http-auth`'s `ChallengeParser` |
| RFC 2326 RTSP 1.0 §14 and §16 | RFC 2326 (vendored `specs/rfc2326_rtsp.txt`) | RTSP reuses Basic and Digest; only the request URI differs | Full | Used by `rtsp-runtime` |
| Forwarded (reverse-proxy trust) | Workspace scheme, not a standard | Trusts a configured header (default convention `X-Forwarded-User`) | Full, server only | Safe only behind a proxy that strips client-supplied copies |
| Signed URLs (HMAC-SHA256 `exp`/`kid`/`sig`, optional `ip`) | Workspace scheme, CDN-style, not a standard | Mint and verify, key rotation across `kid`s, optional IP scope, constant-time compare | Full, server only | `signed_url::SignedUrlKeySet` |

## Features

None. The crate has no Cargo features, is `std`, and has `#![forbid(unsafe_code)]`.

## Quick start

A loopback of the whole flow: an origin issues a Digest challenge, a client answers it, the origin verifies.

```rust
use broadcast_auth::{AuthResult, Authenticator, Credentials, RequestContext, Verifier};

let creds = Credentials::new("admin", "12345");
let verifier = Verifier::new(creds.clone(), "cameras");

// 401 response: the server renders the challenge.
let challenge = verifier.challenge();
assert!(challenge.starts_with("Digest "));

// Client answers it and keeps the Authenticator for later requests.
let mut client = Authenticator::from_challenge(&challenge, creds).unwrap();
let authorization = client
    .authorization(&RequestContext::new("DESCRIBE", "rtsp://cam/stream"))
    .unwrap();

// Server verifies the retried request.
let headers = [("Authorization", authorization.as_str())];
let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/stream").with_headers(&headers);
assert_eq!(verifier.verify(&ctx), AuthResult::Ok);

// A wrong password is refused.
let mut bad = Authenticator::from_challenge(&challenge, Credentials::new("admin", "nope")).unwrap();
let wrong = bad.authorization(&RequestContext::new("DESCRIBE", "rtsp://cam/stream")).unwrap();
let headers = [("Authorization", wrong.as_str())];
let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/stream").with_headers(&headers);
assert_eq!(verifier.verify(&ctx), AuthResult::Unauthorized);
```

Signed URLs, with no `Authorization` header at all:

```rust
use broadcast_auth::{AuthResult, RequestContext, SignedUrlKeySet, Verifier};

let keys = SignedUrlKeySet::new([("key-1".to_string(), vec![0u8; 32])]).unwrap();
let query = keys.sign("key-1", "/stream/media.m3u8", 4_000_000_000, None).unwrap();
let verifier = Verifier::signed_url(keys);
let uri = format!("/stream/media.m3u8?{query}");
assert_eq!(verifier.verify(&RequestContext::new("GET", &uri)), AuthResult::Ok);
```

Runnable programs: `cargo run -p broadcast-auth --example client_respond` and `--example server_verify`.

## What is implemented

- **Client:** `Credentials::new` (answers whichever of Basic or Digest the challenge advertises) and `Credentials::bearer`; `Authenticator::from_challenge` plus `authorization` for session reuse; one-shot `respond`.
- **Server:** `Verifier::new` (scheme chosen by the `Credentials` variant: Basic, Digest or Bearer), `Verifier::forwarded`, `Verifier::signed_url`, `challenge` / `challenge_for`, `verify` returning `AuthResult`, `with_clock` and `with_digest_nc_capacity` for tests and tuning.
- **`RequestContext`:** method, URI, optional body and headers (case-insensitive `header` lookup), peer address.
- **Security properties:** `Credentials`, `Verifier`, nonce keys and `SignedUrlKeySet` zeroize on drop; comparisons use `subtle::ConstantTimeEq`; Digest nonces are stateless-signed and replay-windowed; a `secret_debug_guard` test forbids derived `Debug` over secrets.

## Design notes

Delegates challenge parsing and Digest hashing to `http-auth`; types are `#[non_exhaustive]` where they may grow; errors are structured (`Error`); a lexical `dehandroll_guard` test keeps hand-rolled HTTP or base64 out of the source.

## Related crates

Consumed by `rtsp-runtime` (re-exports `Credentials`), `hls-runtime` (client adapter) and `multimux` (input adapters and the shared output-auth gate). It depends on no other workspace crate.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
