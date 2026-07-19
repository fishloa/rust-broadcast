//! The scheme-agnostic credential model shared by every auth-facing client.

/// Credentials for one of the supported auth schemes.
///
/// `Basic` and `Digest` both carry a username/password and are treated
/// identically by [`crate::respond`]/[`crate::Authenticator`]: which wire
/// scheme is actually used is decided by the server's challenge, not by which
/// variant was constructed (RFC 7235 content negotiation — a server may offer
/// either, or both, in `WWW-Authenticate`). Use [`Credentials::new`] for the
/// common password case; construct `Basic`/`Digest` directly only when the
/// caller must pin one scheme.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credentials {
    /// Username/password for HTTP Basic auth (RFC 7617) — or RTSP's reuse of
    /// it (RFC 2326 §14/§16).
    Basic {
        /// Account username.
        username: String,
        /// Account password.
        password: String,
    },
    /// Username/password for HTTP Digest auth (RFC 7616) — or RTSP's reuse of
    /// it (RFC 2326 §14).
    Digest {
        /// Account username.
        username: String,
        /// Account password.
        password: String,
    },
    /// A bearer token (RFC 6750) — sent verbatim as `Authorization: Bearer
    /// <token>` with no challenge round-trip required.
    Bearer {
        /// The opaque bearer token.
        token: String,
    },
}

impl Credentials {
    /// Convenience constructor for the common password-based case.
    ///
    /// Does not commit to Basic or Digest: [`crate::respond`]/
    /// [`crate::Authenticator::from_challenge`] answer whichever scheme the
    /// server's `WWW-Authenticate` challenge advertises. Internally this
    /// builds a `Digest` value (the superset — `http-auth`'s challenge
    /// parser inspects the challenge itself, not this variant, to pick the
    /// wire scheme), so `new("u", "p")` behaves identically to a
    /// hand-constructed `Basic`/`Digest` with the same username/password.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Credentials::Digest {
            username: username.into(),
            password: password.into(),
        }
    }

    /// Constructs a bearer-token credential (RFC 6750).
    pub fn bearer(token: impl Into<String>) -> Self {
        Credentials::Bearer {
            token: token.into(),
        }
    }

    /// Returns the `(username, password)` pair for a password-based scheme,
    /// or `None` for `Bearer`.
    pub(crate) fn username_password(&self) -> Option<(&str, &str)> {
        match self {
            Credentials::Basic { username, password }
            | Credentials::Digest { username, password } => Some((username, password)),
            Credentials::Bearer { .. } => None,
        }
    }
}
