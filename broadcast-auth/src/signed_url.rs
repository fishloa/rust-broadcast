//! HMAC-signed URL access control (issue #747) — CDN-style, short-lived,
//! tamper-proof query-string tokens that gate a media egress route without
//! the caller carrying a credential header at all.
//!
//! # Wire form
//!
//! Query parameters on the media URL:
//!
//! ```text
//! ?exp=<unix-seconds>&kid=<key-id>&sig=<base64url-nopad>[&ip=<addr>]
//! ```
//!
//! - `exp` — an absolute Unix timestamp (seconds). The token is invalid once
//!   `now > exp`; there is no clock-skew grace period, by design (the caller
//!   sets the window when minting the token).
//! - `kid` — selects which of [`SignedUrlKeySet`]'s configured secrets
//!   verifies this token, so keys can rotate (multiple stay valid at once)
//!   without invalidating URLs already handed out under an older key. An
//!   unrecognised `kid` is a rejection, not a fallback to any other key.
//! - `sig` — HMAC-SHA256 over the canonical string below, base64url-encoded
//!   **without padding** (RFC 4648 §5, no trailing `=`).
//! - `ip` — optional. When present, it must equal the connection's actual
//!   peer address (compared as an [`core::net::IpAddr`], ignoring any port —
//!   see [`Verifier::verify`](crate::Verifier::verify)'s signed-URL arm);
//!   when absent, no IP check is made. Either way it is part of the signed
//!   string, so an attacker can neither add nor strip it from a token they
//!   didn't mint.
//!
//! # Canonical string
//!
//! The exact newline-separated bytes the HMAC is computed over, in this
//! order:
//!
//! ```text
//! <path>\n<exp>\n<ip-or-empty-string>
//! ```
//!
//! `<path>` is the request path *without* its query string (whatever the
//! caller passes to [`SignedUrlKeySet::sign`] — for a `broadcast_auth`-gated
//! origin, the same pre-rewrite request path a
//! [`crate::RequestContext::uri`] carries). **The path is always signed.**
//! Without it, a token minted for one route would verify against every other
//! route the same keyset gates — the entire point of naming the resource in
//! the signature. `<ip-or-empty-string>` is the canonical [`core::net::IpAddr`]
//! `Display` form (not the raw query-string bytes — see
//! [`SignedUrlKeySet::sign`]'s docs for why), or an empty string when no `ip`
//! is bound.
//!
//! # Security properties
//!
//! - **Constant-time compare**: the decoded `sig` bytes are compared against
//!   the freshly recomputed HMAC via [`subtle::ConstantTimeEq`] — never a
//!   short-circuiting `==` on a MAC (a classic timing oracle).
//! - **Uniform rejection**: [`crate::Verifier::verify`] folds every failure
//!   mode of this scheme (missing/unparseable `exp`, unknown `kid`,
//!   missing/malformed `sig`, expired, wrong signature, wrong `ip`) into the
//!   same [`crate::AuthResult::Unauthorized`] — nothing here tells a caller
//!   *which* check failed.
//! - **Minimum secret length**: [`SignedUrlKeySet::new`] rejects any secret
//!   shorter than [`SignedUrlKeySet::MIN_SECRET_LEN`] at construction time
//!   (a config/setup error), not per-request.
//! - **No secret/signature logging**: neither this module nor
//!   [`SignedUrlKeySet`]'s `Debug` impl ever renders a secret or a minted
//!   signature.

use core::net::IpAddr;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::{Error, Result};
use crate::request::RequestContext;

/// One or more `(kid, secret)` HMAC keys valid for signed-URL verification —
/// see the module docs for the wire form and canonical string, and
/// [`crate::Verifier::signed_url`] for wiring one into a [`crate::Verifier`].
///
/// Multiple keys may be active simultaneously (key rotation, issue #747):
/// verification looks the token's `kid` up in this set and rejects an
/// unrecognised one rather than falling back to any other configured key.
pub struct SignedUrlKeySet {
    keys: Vec<(String, Vec<u8>)>,
}

impl SignedUrlKeySet {
    /// The minimum accepted secret length, in bytes (32 — enough entropy that
    /// brute-forcing the HMAC key is infeasible). Enforced by [`Self::new`]
    /// at construction time, never per-request.
    pub const MIN_SECRET_LEN: usize = 32;

    /// Builds a keyset from `(kid, secret)` pairs.
    ///
    /// Rejects (before constructing anything) the first key whose `secret`
    /// is shorter than [`Self::MIN_SECRET_LEN`] bytes, with
    /// [`Error::SignedUrlKeyTooShort`] naming the offending `kid` and
    /// lengths — this is a setup/config-time error, not something a request
    /// can trigger.
    ///
    /// Duplicate `kid`s are not rejected; the internal lookup (used by both
    /// [`Self::sign`] and verification) returns the *first* match, so a
    /// duplicate is effectively shadowed rather than causing ambiguity.
    pub fn new(keys: impl IntoIterator<Item = (String, Vec<u8>)>) -> Result<Self> {
        let keys: Vec<(String, Vec<u8>)> = keys.into_iter().collect();
        for (kid, secret) in &keys {
            if secret.len() < Self::MIN_SECRET_LEN {
                return Err(Error::SignedUrlKeyTooShort {
                    kid: kid.clone(),
                    min: Self::MIN_SECRET_LEN,
                    actual: secret.len(),
                });
            }
        }
        Ok(SignedUrlKeySet { keys })
    }

    fn secret_for(&self, kid: &str) -> Option<&[u8]> {
        self.keys
            .iter()
            .find(|(k, _)| k == kid)
            .map(|(_, s)| s.as_slice())
    }

    /// Signing helper: mints the query-string portion of a signed URL for
    /// `kid`/`path`/`exp`/`ip` — `?`-prefix and the resource path are the
    /// caller's own to assemble (e.g. `format!("{path}?{query}")`), since
    /// this returns just the `exp=...&kid=...&sig=...[&ip=...]` part.
    ///
    /// `path` must be exactly the request path the eventual request's
    /// [`RequestContext::uri`] carries (sans query string) — see the module
    /// docs' canonical-string rule. `ip`, if given, is rendered via
    /// [`IpAddr`]'s canonical `Display` form in both the signed string and
    /// the `ip` query parameter — not whatever string representation a
    /// caller might otherwise have on hand — so that
    /// [`crate::Verifier::verify`]'s re-parse-then-re-render of the `ip` it
    /// receives back always reproduces the exact bytes that were signed.
    ///
    /// Fails with [`Error::UnknownSignedUrlKeyId`] if `kid` is not in this
    /// keyset. This is the *signing* side (used by tests and by whatever
    /// mints tokens for real clients) — [`crate::Verifier::verify`] never
    /// returns this error; an unknown `kid` there is folded into the same
    /// [`crate::AuthResult::Unauthorized`] as every other rejection reason.
    pub fn sign(&self, kid: &str, path: &str, exp: u64, ip: Option<IpAddr>) -> Result<String> {
        let secret = self
            .secret_for(kid)
            .ok_or_else(|| Error::UnknownSignedUrlKeyId(kid.to_string()))?;
        let sig = URL_SAFE_NO_PAD.encode(hmac_sha256(secret, &canonical_string(path, exp, ip)));
        let mut query = format!("exp={exp}&kid={kid}&sig={sig}");
        if let Some(ip) = ip {
            query.push_str(&format!("&ip={ip}"));
        }
        Ok(query)
    }
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): every entry carries a
/// secret that must never render verbatim — only the configured `kid`s are
/// shown, which is exactly what's useful for diagnosing a rotation.
impl core::fmt::Debug for SignedUrlKeySet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SignedUrlKeySet")
            .field(
                "kids",
                &self
                    .keys
                    .iter()
                    .map(|(k, _)| k.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// The exact newline-separated bytes the HMAC is computed over — see the
/// module docs' "Canonical string" section. `ip` is rendered via
/// [`IpAddr`]'s `Display` (canonical form), never the raw query-string
/// bytes, so sign and verify always agree on the same representation.
fn canonical_string(path: &str, exp: u64, ip: Option<IpAddr>) -> String {
    match ip {
        Some(ip) => format!("{path}\n{exp}\n{ip}"),
        None => format!("{path}\n{exp}\n"),
    }
}

/// HMAC-SHA256(`secret`, `message`) — `new_from_slice` never fails for HMAC
/// (it accepts a key of any length, padding/hashing it per RFC 2104 §2 if
/// need be), so the only way this could panic is a `hmac`/`sha2` internal
/// bug, not anything caller-controlled.
fn hmac_sha256(secret: &[u8], message: &str) -> Vec<u8> {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("HMAC accepts a key of any length");
    mac.update(message.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Splits `uri` (a [`RequestContext::uri`], e.g. `/stream/media.m3u8?exp=…`)
/// into `(path, query)`; `query` is `""` when there is no `?` at all.
fn split_path_and_query(uri: &str) -> (&str, &str) {
    match uri.split_once('?') {
        Some((path, query)) => (path, query),
        None => (uri, ""),
    }
}

/// Looks up `key` in a raw (unparsed) query string — `a=1&b=2` — returning
/// the *first* matching value, or `None` if `key` is absent. Never panics on
/// malformed input: a pair with no `=` is simply skipped.
fn query_get<'q>(query: &'q str, key: &str) -> Option<&'q str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// The current wall-clock time as Unix seconds. `unwrap_or(0)` on a
/// pre-epoch clock is unreachable on any real system but is not itself a
/// security-relevant fallback: it can only make [`verify`] *stricter*
/// (`0` fails every non-zero `exp` as already expired), never laxer.
fn current_unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Verifies a signed-URL token carried in `ctx.uri`'s query string against
/// `keys` — the implementation behind
/// [`crate::Verifier::verify`]'s `SignedUrl` arm. See the module docs for
/// the wire form, canonical string, and rejection semantics.
///
/// Every one of `exp`/`kid`/`sig` missing or malformed, `kid` unknown, `sig`
/// undecodable, or `ip` unparseable is treated identically: `false`. Both the
/// expiry check and the signature check are always computed (into their own
/// `bool`s) before being combined — neither is allowed to gate access on its
/// own; a correctly-signed-but-expired token and a fresh-but-wrongly-signed
/// token are both rejected.
pub(crate) fn verify(ctx: &RequestContext<'_>, keys: &SignedUrlKeySet) -> bool {
    let (path, query) = split_path_and_query(ctx.uri);

    let Some(exp) = query_get(query, "exp").and_then(|s| s.parse::<u64>().ok()) else {
        return false;
    };
    let Some(kid) = query_get(query, "kid").filter(|s| !s.is_empty()) else {
        return false;
    };
    let Some(sig) = query_get(query, "sig").filter(|s| !s.is_empty()) else {
        return false;
    };
    let Some(secret) = keys.secret_for(kid) else {
        return false;
    };
    let ip: Option<IpAddr> = match query_get(query, "ip") {
        Some(raw) => match raw.parse::<IpAddr>() {
            Ok(ip) => Some(ip),
            Err(_) => return false,
        },
        None => None,
    };
    let Ok(decoded_sig) = URL_SAFE_NO_PAD.decode(sig) else {
        return false;
    };

    let expected = hmac_sha256(secret, &canonical_string(path, exp, ip));
    let sig_ok = bool::from(decoded_sig.as_slice().ct_eq(expected.as_slice()));
    let not_expired = current_unix_time() <= exp;
    let ip_ok = match ip {
        Some(bound_ip) => ctx.peer_addr.map(|p| p.ip()) == Some(bound_ip),
        None => true,
    };

    sig_ok && not_expired && ip_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuthResult, Verifier};

    const SECRET_A: &[u8; 32] = b"01234567890123456789012345678901";
    const SECRET_B: &[u8; 32] = b"abcdefghijabcdefghijabcdefghij01";

    fn keyset() -> SignedUrlKeySet {
        SignedUrlKeySet::new([
            ("key-a".to_string(), SECRET_A.to_vec()),
            ("key-b".to_string(), SECRET_B.to_vec()),
        ])
        .unwrap()
    }

    fn far_future() -> u64 {
        current_unix_time() + 3600
    }

    fn ctx_for(uri: &str) -> RequestContext<'_> {
        RequestContext::new("GET", uri)
    }

    // --- construction ---

    #[test]
    fn new_rejects_secret_shorter_than_min_len() {
        let err = SignedUrlKeySet::new([("k".to_string(), vec![0u8; 31])]).unwrap_err();
        assert!(matches!(
            err,
            Error::SignedUrlKeyTooShort {
                min: 32,
                actual: 31,
                ..
            }
        ));
    }

    #[test]
    fn new_accepts_exactly_min_len_secret() {
        SignedUrlKeySet::new([("k".to_string(), vec![0u8; 32])]).unwrap();
    }

    // --- sign/verify round trip (1: valid + unexpired -> allowed) ---

    #[test]
    fn valid_signature_and_unexpired_is_allowed() {
        let keys = keyset();
        let verifier = Verifier::signed_url(keyset());
        let exp = far_future();
        let query = keys.sign("key-a", "/stream/media.m3u8", exp, None).unwrap();
        let uri = format!("/stream/media.m3u8?{query}");
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Ok);
    }

    // --- 2: valid signature + expired -> denied ---

    #[test]
    fn valid_signature_but_expired_is_denied() {
        let keys = keyset();
        let verifier = Verifier::signed_url(keyset());
        let exp = current_unix_time().saturating_sub(60);
        let query = keys.sign("key-a", "/stream/media.m3u8", exp, None).unwrap();
        let uri = format!("/stream/media.m3u8?{query}");
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Unauthorized);
    }

    // --- 3: cross-route replay (path substitution) -> denied ---
    // The single most important test: a signature minted for one route must
    // not verify against a different route on the same origin/keyset.

    #[test]
    fn signature_minted_for_one_path_is_rejected_on_another_path() {
        let keys = keyset();
        let verifier = Verifier::signed_url(keyset());
        let exp = far_future();
        let query = keys
            .sign("key-a", "/route-a/media.m3u8", exp, None)
            .unwrap();
        // Same query string, replayed against a different route's path.
        let uri = format!("/route-b/media.m3u8?{query}");
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Unauthorized);
    }

    // --- 4: tampered exp (extended without re-signing) -> denied ---

    #[test]
    fn extending_exp_without_resigning_is_denied() {
        let keys = keyset();
        let verifier = Verifier::signed_url(keyset());
        let original_exp = far_future();
        let query = keys
            .sign("key-a", "/stream/media.m3u8", original_exp, None)
            .unwrap();
        let tampered = query.replacen(
            &format!("exp={original_exp}"),
            &format!("exp={}", original_exp + 1_000_000),
            1,
        );
        assert_ne!(query, tampered, "test setup: exp must actually change");
        let uri = format!("/stream/media.m3u8?{tampered}");
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Unauthorized);
    }

    // --- 5: unknown kid -> denied ---

    #[test]
    fn unknown_kid_is_denied() {
        let keys = keyset();
        let verifier = Verifier::signed_url(keyset());
        let exp = far_future();
        let query = keys.sign("key-a", "/stream/media.m3u8", exp, None).unwrap();
        let tampered = query.replacen("kid=key-a", "kid=nonexistent-key", 1);
        let uri = format!("/stream/media.m3u8?{tampered}");
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Unauthorized);
    }

    // --- 6/7: IP scoping ---

    #[test]
    fn ip_scoped_token_from_different_peer_is_denied() {
        let keys = keyset();
        let verifier = Verifier::signed_url(keyset());
        let exp = far_future();
        let bound_ip: IpAddr = "203.0.113.7".parse().unwrap();
        let query = keys
            .sign("key-a", "/stream/media.m3u8", exp, Some(bound_ip))
            .unwrap();
        let uri = format!("/stream/media.m3u8?{query}");
        let other_peer: std::net::SocketAddr = "198.51.100.9:443".parse().unwrap();
        let ctx = ctx_for(&uri).with_peer_addr(other_peer);
        assert_eq!(verifier.verify(&ctx), AuthResult::Unauthorized);
    }

    #[test]
    fn ip_scoped_token_from_correct_peer_is_allowed() {
        let keys = keyset();
        let verifier = Verifier::signed_url(keyset());
        let exp = far_future();
        let bound_ip: IpAddr = "203.0.113.7".parse().unwrap();
        let query = keys
            .sign("key-a", "/stream/media.m3u8", exp, Some(bound_ip))
            .unwrap();
        let uri = format!("/stream/media.m3u8?{query}");
        // Same IP, different (irrelevant) port — the port must not matter.
        let same_ip_peer: std::net::SocketAddr = "203.0.113.7:54321".parse().unwrap();
        let ctx = ctx_for(&uri).with_peer_addr(same_ip_peer);
        assert_eq!(verifier.verify(&ctx), AuthResult::Ok);
    }

    #[test]
    fn unscoped_token_is_allowed_regardless_of_peer() {
        let keys = keyset();
        let verifier = Verifier::signed_url(keyset());
        let exp = far_future();
        let query = keys.sign("key-a", "/stream/media.m3u8", exp, None).unwrap();
        let uri = format!("/stream/media.m3u8?{query}");
        let peer: std::net::SocketAddr = "198.51.100.9:443".parse().unwrap();
        let ctx = ctx_for(&uri).with_peer_addr(peer);
        assert_eq!(verifier.verify(&ctx), AuthResult::Ok);
    }

    // --- 8: missing/empty/malformed sig -> denied, no panic ---

    #[test]
    fn missing_sig_is_denied() {
        let verifier = Verifier::signed_url(keyset());
        let uri = format!("/stream/media.m3u8?exp={}&kid=key-a", far_future());
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Unauthorized);
    }

    #[test]
    fn empty_sig_is_denied() {
        let verifier = Verifier::signed_url(keyset());
        let uri = format!("/stream/media.m3u8?exp={}&kid=key-a&sig=", far_future());
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Unauthorized);
    }

    #[test]
    fn malformed_base64_sig_is_denied_not_panicking() {
        let verifier = Verifier::signed_url(keyset());
        let uri = format!(
            "/stream/media.m3u8?exp={}&kid=key-a&sig=not-valid-base64!!!",
            far_future()
        );
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Unauthorized);
    }

    #[test]
    fn missing_exp_is_denied() {
        let verifier = Verifier::signed_url(keyset());
        let uri = "/stream/media.m3u8?kid=key-a&sig=AAAA".to_string();
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Unauthorized);
    }

    #[test]
    fn unparseable_exp_is_denied() {
        let verifier = Verifier::signed_url(keyset());
        let uri = "/stream/media.m3u8?exp=not-a-number&kid=key-a&sig=AAAA".to_string();
        assert_eq!(verifier.verify(&ctx_for(&uri)), AuthResult::Unauthorized);
    }

    #[test]
    fn empty_query_string_is_denied_not_panicking() {
        let verifier = Verifier::signed_url(keyset());
        assert_eq!(
            verifier.verify(&ctx_for("/stream/media.m3u8")),
            AuthResult::Unauthorized
        );
        assert_eq!(
            verifier.verify(&ctx_for("/stream/media.m3u8?")),
            AuthResult::Unauthorized
        );
    }

    // --- 9: key rotation ---

    #[test]
    fn both_active_keys_are_accepted_until_one_is_retired() {
        let keys = keyset();
        let exp = far_future();
        let query_a = keys.sign("key-a", "/stream/media.m3u8", exp, None).unwrap();
        let query_b = keys.sign("key-b", "/stream/media.m3u8", exp, None).unwrap();

        let verifier = Verifier::signed_url(keyset());
        assert_eq!(
            verifier.verify(&ctx_for(&format!("/stream/media.m3u8?{query_a}"))),
            AuthResult::Ok,
            "key-a must verify while both keys are active"
        );
        assert_eq!(
            verifier.verify(&ctx_for(&format!("/stream/media.m3u8?{query_b}"))),
            AuthResult::Ok,
            "key-b must verify while both keys are active"
        );

        // Retire key-b: a fresh keyset/verifier carrying only key-a.
        let retired_keyset =
            SignedUrlKeySet::new([("key-a".to_string(), SECRET_A.to_vec())]).unwrap();
        let verifier_after_rotation = Verifier::signed_url(retired_keyset);
        assert_eq!(
            verifier_after_rotation.verify(&ctx_for(&format!("/stream/media.m3u8?{query_a}"))),
            AuthResult::Ok,
            "key-a must still verify after key-b is retired"
        );
        assert_eq!(
            verifier_after_rotation.verify(&ctx_for(&format!("/stream/media.m3u8?{query_b}"))),
            AuthResult::Unauthorized,
            "a token signed by the retired key-b must now be rejected"
        );
    }

    // --- signing helper errors ---

    #[test]
    fn sign_with_unknown_kid_is_a_structured_error() {
        let keys = keyset();
        let err = keys
            .sign("no-such-key", "/stream/media.m3u8", far_future(), None)
            .unwrap_err();
        assert!(matches!(err, Error::UnknownSignedUrlKeyId(k) if k == "no-such-key"));
    }

    // --- misc unit coverage ---

    #[test]
    fn split_path_and_query_handles_no_query() {
        assert_eq!(split_path_and_query("/a/b"), ("/a/b", ""));
        assert_eq!(split_path_and_query("/a/b?x=1"), ("/a/b", "x=1"));
    }

    #[test]
    fn query_get_skips_malformed_pairs_without_panicking() {
        assert_eq!(query_get("a=1&garbage&b=2", "b"), Some("2"));
        assert_eq!(query_get("a=1&garbage&b=2", "garbage"), None);
        assert_eq!(query_get("", "a"), None);
    }

    #[test]
    fn debug_never_leaks_secret_bytes() {
        let keys = keyset();
        let debug = format!("{keys:?}");
        assert!(debug.contains("key-a"), "kid should render: {debug}");
        assert!(debug.contains("key-b"), "kid should render: {debug}");
        assert!(
            !debug.contains(std::str::from_utf8(SECRET_A).unwrap()),
            "secret leaked: {debug}"
        );
    }
}
