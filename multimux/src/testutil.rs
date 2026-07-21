//! Test-only mock HTTP auth server (issue #663 "Finish client-side
//! multi-scheme auth"): an axum middleware layer that gates a router behind
//! Basic (RFC 7617), Digest (RFC 7616), or Bearer (RFC 6750) auth, for
//! `source::ts_http`/`source::hls_pull`'s own loopback tests to drive the
//! *real* `TsHttpSource`/`HlsPullSource` (and therefore the real
//! `source::http_auth`/`ll_hls_runtime::client::tokio_client` challenge-
//! response code) against.
//!
//! Digest verification is a real, independent RFC 7616 §3.4.1 computation
//! (`HA1 = MD5(username:realm:password)`, `HA2 = MD5(method:uri)`,
//! `response = MD5(HA1:nonce:nc:cnonce:qop:HA2)`, `qop=auth`/`algorithm=MD5`
//! only — the one shape [`crate::source::http_auth`]'s client side answers)
//! — not a byte-literal comparison against a precomputed expected header —
//! so a client sending the wrong password (or the wrong scheme entirely)
//! genuinely fails here, the same way a real Digest-speaking origin would
//! reject it. `http-auth` (this workspace's Digest client library) has no
//! server-side counterpart to reuse, so this is a small, deliberate,
//! test-only exception to "no hand-rolled digest in multimux" — the
//! production challenge/response code all still lives in `broadcast-auth`.

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use md5::{Digest as _, Md5};

/// Which scheme a [`require_auth`]-wrapped router demands, and the
/// credentials it demands them for.
#[derive(Clone)]
pub(crate) enum MockAuthScheme {
    /// HTTP Basic (RFC 7617).
    Basic { username: String, password: String },
    /// HTTP Digest (RFC 7616), `qop=auth`/`algorithm=MD5` — the shape
    /// [`crate::source::http_auth`]'s client answers.
    Digest {
        username: String,
        password: String,
        realm: String,
        nonce: String,
    },
    /// Bearer (RFC 6750) — config-supplied only, never from URL userinfo.
    Bearer { token: String },
}

impl MockAuthScheme {
    /// The `WWW-Authenticate` challenge value this scheme issues on a
    /// missing/failed `Authorization`.
    fn challenge(&self) -> String {
        match self {
            MockAuthScheme::Basic { .. } => "Basic realm=\"mock\"".to_string(),
            MockAuthScheme::Digest { realm, nonce, .. } => {
                format!("Digest realm=\"{realm}\", nonce=\"{nonce}\", qop=\"auth\", algorithm=MD5")
            }
            MockAuthScheme::Bearer { .. } => "Bearer realm=\"mock\"".to_string(),
        }
    }

    /// Checks an incoming request's `Authorization` header (if any) against
    /// this scheme's credentials.
    fn check(&self, headers: &HeaderMap, method: &str) -> bool {
        let Some(auth) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        else {
            return false;
        };
        match self {
            MockAuthScheme::Basic { username, password } => {
                let expected = format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD
                        .encode(format!("{username}:{password}"))
                );
                auth == expected
            }
            MockAuthScheme::Bearer { token } => auth == format!("Bearer {token}"),
            MockAuthScheme::Digest {
                username,
                password,
                realm,
                nonce,
            } => verify_digest(auth, username, password, realm, nonce, method),
        }
    }
}

/// Parses a `key=value`/`key="value"` Digest `Authorization` header into its
/// fields (RFC 7616 §3.4), then independently recomputes the expected
/// `response` and compares — see the module doc for why this is a real
/// computation, not a literal-string match.
fn verify_digest(
    auth_header: &str,
    expected_username: &str,
    password: &str,
    expected_realm: &str,
    expected_nonce: &str,
    method: &str,
) -> bool {
    let Some(rest) = auth_header.strip_prefix("Digest ") else {
        return false;
    };
    let mut fields = std::collections::HashMap::new();
    for part in rest.split(',') {
        let part = part.trim();
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        fields.insert(key.trim(), value.trim().trim_matches('"'));
    }
    let get = |k: &str| fields.get(k).copied().unwrap_or_default();

    if get("username") != expected_username
        || get("realm") != expected_realm
        || get("nonce") != expected_nonce
    {
        return false;
    }
    let uri = get("uri");
    let nc = get("nc");
    let cnonce = get("cnonce");
    let qop = get("qop");
    let client_response = get("response");
    if uri.is_empty() || nc.is_empty() || cnonce.is_empty() || client_response.is_empty() {
        return false;
    }

    let ha1 = md5_hex(format!("{expected_username}:{expected_realm}:{password}"));
    let ha2 = md5_hex(format!("{method}:{uri}"));
    let expected_response = md5_hex(format!("{ha1}:{expected_nonce}:{nc}:{cnonce}:{qop}:{ha2}"));
    expected_response == client_response
}

/// Lowercase-hex MD5 digest of `input`.
fn md5_hex(input: String) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Wraps `router` behind `scheme`: every request needs a valid
/// `Authorization` matching `scheme`'s credentials, or gets a `401` +
/// `WWW-Authenticate` challenge naming it.
pub(crate) fn require_auth(router: Router, scheme: MockAuthScheme) -> Router {
    router.layer(middleware::from_fn_with_state(scheme, auth_gate))
}

async fn auth_gate(State(scheme): State<MockAuthScheme>, req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    if scheme.check(req.headers(), &method) {
        return next.run(req).await;
    }
    let challenge = scheme.challenge();
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, challenge)],
        Body::empty(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent sanity check of [`verify_digest`] against a known-good
    /// RFC 7616-shaped Digest response, computed by hand from the same
    /// formula this module implements (not copy-pasted from the production
    /// client code) — proves the oracle itself is correct before any
    /// `TsHttpSource`/`HlsPullSource` test relies on it.
    #[test]
    fn verify_digest_accepts_hand_computed_response() {
        let username = "admin";
        let password = "12345";
        let realm = "mock realm";
        let nonce = "abc123nonce";
        let method = "GET";
        let uri = "http://127.0.0.1:9/stream.ts";
        let nc = "00000001";
        let cnonce = "clientnonce";
        let qop = "auth";

        let ha1 = md5_hex(format!("{username}:{realm}:{password}"));
        let ha2 = md5_hex(format!("{method}:{uri}"));
        let response = md5_hex(format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"));

        let header = format!(
            "Digest username=\"{username}\", realm=\"{realm}\", nonce=\"{nonce}\", \
             uri=\"{uri}\", qop={qop}, nc={nc}, cnonce=\"{cnonce}\", response=\"{response}\""
        );
        assert!(verify_digest(
            &header, username, password, realm, nonce, method
        ));
    }

    #[test]
    fn verify_digest_rejects_wrong_password() {
        let username = "admin";
        let realm = "mock realm";
        let nonce = "abc123nonce";
        let method = "GET";
        let uri = "http://127.0.0.1:9/stream.ts";
        let nc = "00000001";
        let cnonce = "clientnonce";
        let qop = "auth";

        // Computed with the WRONG password, as an attacker would.
        let ha1 = md5_hex(format!("{username}:{realm}:wrongpass"));
        let ha2 = md5_hex(format!("{method}:{uri}"));
        let response = md5_hex(format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"));

        let header = format!(
            "Digest username=\"{username}\", realm=\"{realm}\", nonce=\"{nonce}\", \
             uri=\"{uri}\", qop={qop}, nc={nc}, cnonce=\"{cnonce}\", response=\"{response}\""
        );
        assert!(!verify_digest(
            &header, username, "12345", realm, nonce, method
        ));
    }
}
