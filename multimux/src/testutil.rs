//! Test-only mock HTTP auth server (issue #663 "Finish client-side
//! multi-scheme auth"; dedup issue #724): an axum middleware layer that
//! gates a router behind Basic (RFC 7617), Digest (RFC 7616), or Bearer (RFC
//! 6750) auth, for `source::ts_http`/`source::hls_pull`'s own loopback tests
//! to drive the *real* `TsHttpSource`/`HlsPullSource` (and therefore the
//! real `source::http_auth`/`ll_hls_runtime::client::tokio_client` challenge-
//! response code) against.
//!
//! A thin wrapper over the real [`broadcast_auth::Verifier`] — the same
//! production challenge/verify code `crate::origin::output_auth_gate` gates
//! every media output route with (see that function). Before issue #724 this
//! module hand-rolled its own independent RFC 7616 §3.4.1 Digest computation
//! (a small, deliberate exception to "no hand-rolled digest in multimux",
//! since `http-auth` — this workspace's Digest client library — has no
//! server-side counterpart); that computation had no request-target check at
//! all (it hashed whatever `uri` the client claimed, unconditionally), so it
//! predated and never exercised `broadcast_auth::Verifier`'s
//! absolute-vs-origin-form `uri`-match fix (issue #724). Wrapping the real
//! `Verifier` here means the mock server now proves the exact same code path
//! multimux's real output-auth middleware runs, including that fix, rather
//! than a second, independently-behaving oracle.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use broadcast_auth::{AuthResult, Credentials, RequestContext, Verifier};

/// Which scheme a [`require_auth`]-wrapped router demands, and the
/// credentials it demands them for.
///
/// Digest carries no `nonce` field (unlike the pre-#724 hand-rolled version):
/// [`Verifier::new`] generates its own fresh random nonce at construction and
/// holds it for the verifier's whole lifetime (its own module docs' caveat),
/// which is exactly what a real client answering a real challenge needs —
/// the mock server's nonce value itself is never asserted on by any test.
pub(crate) enum MockAuthScheme {
    /// HTTP Basic (RFC 7617).
    Basic { username: String, password: String },
    /// HTTP Digest (RFC 7616), `qop=auth`/`algorithm=MD5` — the shape
    /// [`crate::source::http_auth`]'s client answers.
    Digest {
        username: String,
        password: String,
        realm: String,
    },
    /// Bearer (RFC 6750) — config-supplied only, never from URL userinfo.
    Bearer { token: String },
}

impl MockAuthScheme {
    /// Builds the one [`Verifier`] this scheme's requests are gated by (see
    /// [`require_auth`] for why this is built once, not per-request).
    fn into_verifier(self) -> Verifier {
        match self {
            MockAuthScheme::Basic { username, password } => {
                Verifier::new(Credentials::Basic { username, password }, "mock")
            }
            MockAuthScheme::Digest {
                username,
                password,
                realm,
            } => Verifier::new(Credentials::Digest { username, password }, realm),
            MockAuthScheme::Bearer { token } => Verifier::new(Credentials::bearer(token), "mock"),
        }
    }
}

/// Wraps `router` behind `scheme`: every request needs a valid
/// `Authorization` matching `scheme`'s credentials, or gets a `401` +
/// `WWW-Authenticate` challenge naming it.
///
/// Builds `scheme`'s [`Verifier`] exactly once (wrapped in an `Arc` so axum's
/// `State` extractor can cheaply clone the handle per request) — a Digest
/// verifier's server nonce is generated at construction and must stay stable
/// across the challenge and the client's follow-up retry, so this must not
/// rebuild a fresh `Verifier` (and therefore a fresh nonce) on every request.
pub(crate) fn require_auth(router: Router, scheme: MockAuthScheme) -> Router {
    let verifier = Arc::new(scheme.into_verifier());
    router.layer(middleware::from_fn_with_state(verifier, auth_gate))
}

async fn auth_gate(State(verifier): State<Arc<Verifier>>, req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    let uri = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let headers: Vec<(&str, &str)> = req
        .headers()
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.as_str(), v)))
        .collect();
    let ctx = RequestContext::new(&method, &uri).with_headers(&headers);
    if verifier.verify(&ctx) == AuthResult::Ok {
        return next.run(req).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, verifier.challenge())],
        Body::empty(),
    )
        .into_response()
}
