//! Server-side challenge + verify — the origin half of RFC 7235
//! (`WWW-Authenticate`/`Authorization`), RFC 7617 (Basic), RFC 7616 (Digest),
//! RFC 2326 §14 (RTSP's reuse of the same two schemes), and RFC 6750
//! (Bearer).
//!
//! [`crate::Authenticator`]/[`crate::respond`] are the *client* half: answer
//! a challenge. [`Verifier`] is the other side of the same handshake: an
//! origin (multimux's shared output auth gating every `/{stream}/…` route,
//! issue #663; or any other credentialed origin in this workspace) builds
//! one from a configured [`Credentials`] + realm, calls [`Verifier::challenge`]
//! for the `WWW-Authenticate` value to send on a `401`, and
//! [`Verifier::verify`] to check an incoming `Authorization` header.
//!
//! Promoted from `multimux`'s test-only mock auth server
//! (`multimux::testutil`, issue #663 "Finish client-side multi-scheme
//! auth"): that module's Digest verification was already a real,
//! independent RFC 7616 §3.4.1 computation (not a literal-string match)
//! purely to drive multimux's own client-side tests against something that
//! genuinely rejects wrong credentials. This module is that same
//! computation, promoted into the shared crate so it is the *production*
//! server-side verifier (multimux's output-auth middleware) rather than a
//! test-only fixture, and so no crate hand-rolls a second copy.
//!
//! # Verification per scheme
//!
//! - **Basic** (RFC 7617 §2): the header's base64 payload is decoded and
//!   compared, in constant time, against `"{username}:{password}"`.
//! - **Bearer** (RFC 6750 §2.1): the token is compared, in constant time,
//!   against the configured token.
//! - **Digest** (RFC 7616 §3.4.1): `HA1 = MD5(username:realm:password)`,
//!   `HA2 = MD5(method:uri)`, `response = MD5(HA1:nonce:nc:cnonce:qop:HA2)`
//!   — `qop=auth`/`algorithm=MD5` only (the one shape every client in this
//!   workspace answers) — recomputed and compared, in constant time, against
//!   the client's `response` field. The client's claimed `uri` field must
//!   also match the actual request URI (RFC 7616 §3.4.1: the server "SHOULD
//!   check" this), not merely be internally consistent with its own
//!   `response`.
//!
//! # Nonce handling (replay caveat)
//!
//! A [`Verifier`] built for `Digest` generates one random nonce at
//! construction time and reuses it for the verifier's entire lifetime — it
//! does not rotate per-challenge or track consumed `(nonce, nc)` pairs. This
//! is the "simple server nonce" the design spec calls out as acceptable: it
//! is enough to stop a passive credential-sniffing attacker (the password
//! itself is never sent), but — unlike a nonce-tracking implementation — it
//! does **not** detect a replayed exact request (identical `nc`/`cnonce`)
//! within the verifier's lifetime. Rebuild the `Verifier` (e.g. on process
//! restart) to rotate the nonce.

use base64::Engine;
use md5::{Digest as _, Md5};

use crate::credentials::Credentials;

/// The outcome of [`Verifier::verify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthResult {
    /// The `Authorization` header (or absence of one) satisfies the
    /// verifier's configured credential.
    Ok,
    /// Missing, malformed, or wrong-credential `Authorization` — the caller
    /// should respond `401` with [`Verifier::challenge`].
    Unauthorized,
}

/// Per-scheme state a [`Verifier`] holds — mirrors [`Credentials`] but adds
/// the realm (Basic/Digest) and the one server nonce (Digest) generated at
/// construction (see the module docs' nonce-handling caveat).
enum VerifierScheme {
    Basic {
        username: String,
        password: String,
        realm: String,
    },
    Digest {
        username: String,
        password: String,
        realm: String,
        nonce: String,
    },
    Bearer {
        token: String,
    },
}

/// Challenges + verifies incoming requests against one configured
/// [`Credentials`] (RFC 7235 origin-side auth) — see the module docs.
pub struct Verifier {
    scheme: VerifierScheme,
}

impl Verifier {
    /// Builds a verifier for `credentials`, using `realm` for the
    /// `WWW-Authenticate` challenge (Basic/Digest only — RFC 6750 Bearer has
    /// no realm parameter in this crate's minimal challenge, see
    /// [`Self::challenge`]).
    ///
    /// For `Credentials::Digest`, a fresh random server nonce is generated
    /// now and held for this verifier's whole lifetime (see the module
    /// docs' nonce-handling caveat).
    pub fn new(credentials: Credentials, realm: impl Into<String>) -> Self {
        let realm = realm.into();
        let scheme = match credentials {
            Credentials::Basic { username, password } => VerifierScheme::Basic {
                username,
                password,
                realm,
            },
            Credentials::Digest { username, password } => VerifierScheme::Digest {
                username,
                password,
                realm,
                nonce: generate_nonce(),
            },
            Credentials::Bearer { token } => VerifierScheme::Bearer { token },
        };
        Verifier { scheme }
    }

    /// The `WWW-Authenticate` header value to send on a `401` in response to
    /// a missing/failed [`Self::verify`] call.
    pub fn challenge(&self) -> String {
        match &self.scheme {
            VerifierScheme::Basic { realm, .. } => format!("Basic realm=\"{realm}\""),
            VerifierScheme::Digest { realm, nonce, .. } => {
                format!("Digest realm=\"{realm}\", nonce=\"{nonce}\", qop=\"auth\", algorithm=MD5")
            }
            VerifierScheme::Bearer { .. } => "Bearer".to_string(),
        }
    }

    /// Verifies an incoming request's `Authorization` header (`None` if the
    /// request carried none) against this verifier's configured credential.
    ///
    /// `method`/`uri` are the request's method and request-URI (RTSP request
    /// URI, or HTTP path+query — whichever the caller's protocol uses) —
    /// needed for the Digest `HA2`/`uri`-match check (RFC 7616 §3.4.1);
    /// unused for Basic/Bearer.
    pub fn verify(&self, authorization: Option<&str>, method: &str, uri: &str) -> AuthResult {
        let Some(header) = authorization else {
            return AuthResult::Unauthorized;
        };
        let ok = match &self.scheme {
            VerifierScheme::Basic {
                username, password, ..
            } => verify_basic(header, username, password),
            VerifierScheme::Bearer { token } => verify_bearer(header, token),
            VerifierScheme::Digest {
                username,
                password,
                realm,
                nonce,
            } => verify_digest(header, username, password, realm, nonce, method, uri),
        };
        if ok {
            AuthResult::Ok
        } else {
            AuthResult::Unauthorized
        }
    }
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): every scheme carries a
/// secret (`password`/`token`) that must never render verbatim.
impl core::fmt::Debug for Verifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let scheme = match &self.scheme {
            VerifierScheme::Basic { .. } => "Basic",
            VerifierScheme::Digest { .. } => "Digest",
            VerifierScheme::Bearer { .. } => "Bearer",
        };
        f.debug_struct("Verifier")
            .field("scheme", &scheme)
            .finish_non_exhaustive()
    }
}

/// RFC 7617 §2: decode the base64 payload and compare, in constant time,
/// against `"{username}:{password}"`.
fn verify_basic(header: &str, username: &str, password: &str) -> bool {
    let Some(encoded) = header.strip_prefix("Basic ") else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let expected = format!("{username}:{password}");
    constant_time_eq(&decoded, expected.as_bytes())
}

/// RFC 6750 §2.1: compare the bearer token, in constant time.
fn verify_bearer(header: &str, token: &str) -> bool {
    let Some(sent) = header.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(sent.trim().as_bytes(), token.as_bytes())
}

/// RFC 7616 §3.4.1: parse the `Digest` `Authorization` header's
/// `key=value`/`key="value"` fields, independently recompute the expected
/// `response`, and compare in constant time — `qop=auth`/`algorithm=MD5`
/// only (the one shape every client in this workspace answers).
///
/// Also checks the client's claimed `uri` field against the actual request
/// `uri` (RFC 7616 §3.4.1's SHOULD) rather than only using whatever the
/// client claims to compute `HA2`.
fn verify_digest(
    header: &str,
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    method: &str,
    uri: &str,
) -> bool {
    let Some(rest) = header.strip_prefix("Digest ") else {
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

    if get("username") != username || get("realm") != realm || get("nonce") != nonce {
        return false;
    }
    if get("uri") != uri {
        return false;
    }
    let nc = get("nc");
    let cnonce = get("cnonce");
    let qop = get("qop");
    let client_response = get("response");
    if nc.is_empty() || cnonce.is_empty() || client_response.is_empty() {
        return false;
    }

    let ha1 = md5_hex(format!("{username}:{realm}:{password}"));
    let ha2 = md5_hex(format!("{method}:{uri}"));
    let expected_response = md5_hex(format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"));
    constant_time_eq(expected_response.as_bytes(), client_response.as_bytes())
}

/// Lowercase-hex MD5 digest of `input`.
fn md5_hex(input: String) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Byte-equality that does not short-circuit on the first differing byte —
/// only the *length* check short-circuits (an equal-length requirement is
/// not itself the secret being protected). Guards against a timing
/// side-channel on the password/token/digest-response comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// A fresh 128-bit random server nonce, lowercase-hex encoded — see the
/// module docs' nonce-handling caveat.
fn generate_nonce() -> String {
    let bytes: [u8; 16] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Credentials, RequestContext, respond};

    const REALM: &str = "cameras";

    // --- challenge() shape ---

    #[test]
    fn basic_challenge_names_the_realm() {
        let v = Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        assert_eq!(v.challenge(), "Basic realm=\"cameras\"");
    }

    #[test]
    fn digest_challenge_carries_realm_nonce_qop_algorithm() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let challenge = v.challenge();
        assert!(challenge.starts_with("Digest "), "got: {challenge}");
        for needle in [
            "realm=\"cameras\"",
            "nonce=",
            "qop=\"auth\"",
            "algorithm=MD5",
        ] {
            assert!(
                challenge.contains(needle),
                "missing {needle} in {challenge}"
            );
        }
    }

    #[test]
    fn bearer_challenge_is_bare_scheme_name() {
        let v = Verifier::new(Credentials::bearer("tok"), REALM);
        assert_eq!(v.challenge(), "Bearer");
    }

    #[test]
    fn digest_nonce_is_stable_across_repeated_challenge_calls() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        assert_eq!(
            v.challenge(),
            v.challenge(),
            "nonce must not rotate per-call"
        );
    }

    // --- round trip: a client's respond() to challenge() must verify() Ok ---

    #[test]
    fn basic_respond_to_challenge_verifies_ok() {
        let v = Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let header = respond(
            &v.challenge(),
            &RequestContext::new("GET", "/stream"),
            Credentials::new("admin", "12345"),
        )
        .unwrap();
        assert_eq!(v.verify(Some(&header), "GET", "/stream"), AuthResult::Ok);
    }

    #[test]
    fn digest_respond_to_challenge_verifies_ok() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/live");
        let header = respond(&v.challenge(), &ctx, Credentials::new("admin", "12345")).unwrap();
        assert_eq!(
            v.verify(Some(&header), "DESCRIBE", "rtsp://cam/live"),
            AuthResult::Ok
        );
    }

    #[test]
    fn bearer_respond_to_challenge_verifies_ok() {
        let v = Verifier::new(Credentials::bearer("mytoken123"), REALM);
        let header = respond(
            &v.challenge(),
            &RequestContext::new("GET", "/stream"),
            Credentials::bearer("mytoken123"),
        )
        .unwrap();
        assert_eq!(v.verify(Some(&header), "GET", "/stream"), AuthResult::Ok);
    }

    // --- wrong credentials -> Unauthorized (must BITE) ---

    #[test]
    fn basic_wrong_password_is_unauthorized() {
        let v = Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let header = respond(
            &v.challenge(),
            &RequestContext::new("GET", "/stream"),
            Credentials::new("admin", "WRONG"),
        )
        .unwrap();
        assert_eq!(
            v.verify(Some(&header), "GET", "/stream"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn digest_wrong_password_is_unauthorized() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/live");
        let header = respond(&v.challenge(), &ctx, Credentials::new("admin", "WRONG")).unwrap();
        assert_eq!(
            v.verify(Some(&header), "DESCRIBE", "rtsp://cam/live"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn digest_mismatched_request_uri_is_unauthorized() {
        // A digest response computed for one URI must not verify against a
        // different URI the caller passes to `verify` (RFC 7616 SHOULD-check
        // that the header's `uri` matches the actual request).
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/live");
        let header = respond(&v.challenge(), &ctx, Credentials::new("admin", "12345")).unwrap();
        assert_eq!(
            v.verify(Some(&header), "DESCRIBE", "rtsp://cam/OTHER"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn bearer_wrong_token_is_unauthorized() {
        let v = Verifier::new(Credentials::bearer("right-token"), REALM);
        let header = respond(
            &v.challenge(),
            &RequestContext::new("GET", "/stream"),
            Credentials::bearer("wrong-token"),
        )
        .unwrap();
        assert_eq!(
            v.verify(Some(&header), "GET", "/stream"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn missing_authorization_header_is_unauthorized() {
        let v = Verifier::new(Credentials::bearer("tok"), REALM);
        assert_eq!(v.verify(None, "GET", "/stream"), AuthResult::Unauthorized);
    }

    #[test]
    fn wrong_scheme_header_is_unauthorized() {
        // A Basic-configured verifier must reject a Bearer-shaped header
        // (and vice versa) rather than mis-parsing it as a match.
        let v = Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        assert_eq!(
            v.verify(Some("Bearer sometoken"), "GET", "/stream"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn constant_time_eq_matches_naive_equality() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer-string"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn debug_never_leaks_password_or_token() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "supersecret".into(),
            },
            REALM,
        );
        let debug = format!("{v:?}");
        assert!(!debug.contains("supersecret"), "debug: {debug}");

        let v = Verifier::new(Credentials::bearer("topsecrettoken"), REALM);
        let debug = format!("{v:?}");
        assert!(!debug.contains("topsecrettoken"), "debug: {debug}");
    }
}
