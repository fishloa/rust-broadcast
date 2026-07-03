//! Authentication wiring — RFC 2326 §14 (RTSP reuses HTTP auth).
//!
//! RTSP shares HTTP's Basic and Digest schemes verbatim (RFC 2326 §16); see
//! [`docs/auth.md`](../docs/auth.md). The heavy lifting — challenge parsing and
//! the digest response computation — is delegated to the [`http_auth`] crate.
//! This module holds the client [`Credentials`] and an [`Authenticator`] that
//! owns the negotiated [`http_auth::PasswordClient`] and computes an
//! `Authorization` header value for each outgoing request.
//!
//! The one RTSP-specific rule: the `uri` used in the digest computation is the
//! RTSP request URI (e.g. `rtsp://host/stream`), not an HTTP URL (RFC 2326 §14).

use crate::error::{Error, Result};
use http_auth::{PasswordClient, PasswordParams};

/// Username/password credentials for RTSP authentication (RFC 2326 §14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    /// Account username.
    pub username: String,
    /// Account password.
    pub password: String,
}

impl Credentials {
    /// Creates credentials from a username and password.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Credentials {
            username: username.into(),
            password: password.into(),
        }
    }
}

/// Holds the negotiated password client for an authenticated RTSP session.
///
/// Constructed from a `WWW-Authenticate` challenge on a `401` response; then
/// [`Authenticator::authorization`] is called for every subsequent request so
/// that Digest `nc`/`cnonce`/`response` advance correctly (RFC 2326 §14 / see
/// [`docs/auth.md`](../docs/auth.md)).
pub struct Authenticator {
    credentials: Credentials,
    client: PasswordClient,
}

impl Authenticator {
    /// Builds an authenticator from the `WWW-Authenticate` challenge value and
    /// the client's credentials.
    ///
    /// Call this again with the fresh challenge when the server re-challenges
    /// with `stale=true`, to pick up the new nonce (RFC 2326 §14).
    pub fn from_challenge(www_authenticate: &str, credentials: Credentials) -> Result<Self> {
        let client = PasswordClient::try_from(www_authenticate)
            .map_err(|e| Error::Auth(format!("parse WWW-Authenticate: {e}")))?;
        Ok(Authenticator {
            credentials,
            client,
        })
    }

    /// Computes the `Authorization` header value for a request to `uri` using
    /// `method`.
    ///
    /// The `uri` MUST be the RTSP request URI (RFC 2326 §14). For Digest, each
    /// call advances the nonce count; the resulting value carries `response=`,
    /// `realm=`, `nonce=`, `uri=`, and (when `qop` is present) `cnonce=`/`nc=`.
    pub fn authorization(&mut self, method: &str, uri: &str) -> Result<String> {
        self.client
            .respond(&PasswordParams {
                username: &self.credentials.username,
                password: &self.credentials.password,
                uri,
                method,
                body: Some(&[]),
            })
            .map_err(|e| Error::Auth(format!("compute Authorization: {e}")))
    }
}

impl core::fmt::Debug for Authenticator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // PasswordClient is not Debug; avoid leaking the password.
        f.debug_struct("Authenticator")
            .field("username", &self.credentials.username)
            .finish_non_exhaustive()
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
            .authorization("DESCRIBE", "rtsp://camera.example.com/live")
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
        let value = auth.authorization("DESCRIBE", "rtsp://c/live").unwrap();
        assert!(value.starts_with("Basic "));
        assert_ne!(value, "Basic ");
    }
}
