//! Stateful challenge->response computation across a session.

use http_auth::{PasswordClient, PasswordParams};

use crate::credentials::Credentials;
use crate::error::{Error, Result};
use crate::request::RequestContext;

/// Per-scheme state an [`Authenticator`] carries between calls.
///
/// Basic/Digest need the negotiated [`PasswordClient`] kept alive so Digest's
/// `nc` (nonce count) advances correctly across successive requests in the
/// same session (RFC 7616 §3.3); Bearer is stateless.
enum SchemeState {
    Password(PasswordClient),
    Bearer,
}

/// Negotiates a challenge once, then answers every subsequent request in the
/// session (RFC 7235 `WWW-Authenticate`/`Authorization`; RFC 2326 §14 for
/// RTSP; RFC 6750 for Bearer).
///
/// Construct with [`Authenticator::from_challenge`] on the first `401`/`407`,
/// then call [`Authenticator::authorization`] for every outgoing request —
/// including the immediate retry. Call `from_challenge` again if the server
/// re-challenges with a fresh nonce (`stale=true`) to pick it up.
pub struct Authenticator {
    credentials: Credentials,
    state: SchemeState,
}

impl Authenticator {
    /// Builds an authenticator from a `WWW-Authenticate` challenge value and
    /// the caller's credentials.
    ///
    /// For `Credentials::Basic`/`Digest`, the challenge is parsed by
    /// [`http_auth`], which itself decides the wire scheme (Basic or Digest)
    /// from the challenge content — not from which `Credentials` variant was
    /// passed in. For `Credentials::Bearer`, the challenge value is not
    /// inspected: RFC 6750 needs no challenge round-trip, so the token is
    /// used as-is.
    pub fn from_challenge(www_authenticate: &str, credentials: Credentials) -> Result<Self> {
        let state = match &credentials {
            Credentials::Bearer { .. } => SchemeState::Bearer,
            Credentials::Basic { .. } | Credentials::Digest { .. } => {
                let client = PasswordClient::try_from(www_authenticate)
                    .map_err(|e| Error::ChallengeParse(e.to_string()))?;
                SchemeState::Password(client)
            }
        };
        Ok(Authenticator { credentials, state })
    }

    /// Computes the `Authorization` header value for `ctx`.
    ///
    /// For Basic/Digest this advances the Digest nonce count on every call
    /// (RFC 7616 §3.3); for Bearer it always returns `Bearer <token>` (RFC
    /// 6750).
    pub fn authorization(&mut self, ctx: &RequestContext<'_>) -> Result<String> {
        match &mut self.state {
            SchemeState::Bearer => {
                let Credentials::Bearer { token } = &self.credentials else {
                    unreachable!("SchemeState::Bearer only paired with Credentials::Bearer")
                };
                Ok(format!("Bearer {token}"))
            }
            SchemeState::Password(client) => {
                let (username, password) = self
                    .credentials
                    .username_password()
                    .expect("SchemeState::Password only paired with Basic/Digest credentials");
                client
                    .respond(&PasswordParams {
                        username,
                        password,
                        uri: ctx.uri,
                        method: ctx.method,
                        body: ctx.body,
                    })
                    .map_err(|e| Error::ResponseCompute(e.to_string()))
            }
        }
    }
}

impl core::fmt::Debug for Authenticator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // PasswordClient is not Debug; avoid leaking the password/token too.
        let scheme = match &self.credentials {
            Credentials::Basic { .. } => "Basic",
            Credentials::Digest { .. } => "Digest",
            Credentials::Bearer { .. } => "Bearer",
        };
        f.debug_struct("Authenticator")
            .field("scheme", &scheme)
            .finish_non_exhaustive()
    }
}

/// One-shot challenge->response: computes the `Authorization` value for a
/// single request without keeping an [`Authenticator`] around.
///
/// Prefer [`Authenticator`] when the same credentials answer multiple
/// requests in one session (Digest's `nc` must advance) — this is a thin
/// convenience over `Authenticator::from_challenge(..)?.authorization(..)`.
pub fn respond(
    www_authenticate: &str,
    ctx: &RequestContext<'_>,
    credentials: Credentials,
) -> Result<String> {
    Authenticator::from_challenge(www_authenticate, credentials)?.authorization(ctx)
}
