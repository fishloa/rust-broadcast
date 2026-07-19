//! Authentication wiring — RFC 2326 §14 (RTSP reuses HTTP auth).
//!
//! RTSP shares HTTP's Basic and Digest schemes verbatim (RFC 2326 §16); see
//! [`docs/auth.md`](../docs/auth.md). The [`Credentials`] model and the
//! challenge-parsing/response-computation logic now live in the shared
//! [`broadcast_auth`] crate — re-exported here so `rtsp-runtime` callers keep
//! using `rtsp_runtime::Credentials`/`Authenticator` unchanged. This crate
//! contributes only the one RTSP-specific rule: the `uri` used in the digest
//! computation is the RTSP request URI (e.g. `rtsp://host/stream`), not an
//! HTTP URL (RFC 2326 §14) — [`client::ClientSession`](crate::client::ClientSession)
//! passes it through as-is.
//!
//! Sharing this logic with HTTP clients (rather than re-implementing it per
//! client) is the point: `broadcast-auth` also backs `multimux`'s HTTP input
//! adapters (TS-over-HTTP, HLS-pull) and adds Bearer (RFC 6750), which this
//! crate did not previously support.

pub use broadcast_auth::{Authenticator, Credentials, RequestContext};

use crate::error::Error;

impl From<broadcast_auth::Error> for Error {
    fn from(e: broadcast_auth::Error) -> Self {
        Error::Auth(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHALLENGE: &str = "Digest realm=\"IP Camera\",nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\",qop=\"auth\",algorithm=MD5";

    #[test]
    fn digest_authorization_contains_required_fields() {
        let mut auth =
            Authenticator::from_challenge(CHALLENGE, Credentials::new("admin", "12345")).unwrap();
        let value = auth
            .authorization(&RequestContext::new(
                "DESCRIBE",
                "rtsp://camera.example.com/live",
            ))
            .unwrap();
        assert!(value.starts_with("Digest "), "got: {value}");
        for needle in ["response=", "realm=", "nonce=", "uri=", "cnonce=", "nc="] {
            assert!(value.contains(needle), "missing {needle} in {value}");
        }
        assert!(value.contains("uri=\"rtsp://camera.example.com/live\""));
    }

    #[test]
    fn basic_authorization_is_computed() {
        let mut auth = Authenticator::from_challenge(
            "Basic realm=\"IP Camera\"",
            Credentials::new("admin", "12345"),
        )
        .unwrap();
        let value = auth
            .authorization(&RequestContext::new("DESCRIBE", "rtsp://c/live"))
            .unwrap();
        assert!(value.starts_with("Basic "));
        assert_ne!(value, "Basic ");
    }

    #[test]
    fn bearer_authorization_is_computed() {
        let mut auth =
            Authenticator::from_challenge("", Credentials::bearer("mytoken123")).unwrap();
        let value = auth
            .authorization(&RequestContext::new("DESCRIBE", "rtsp://c/live"))
            .unwrap();
        assert_eq!(value, "Bearer mytoken123");
    }
}
