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
#[derive(Clone, PartialEq, Eq)]
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

/// Manual `Debug` (rather than `#[derive(Debug)]`): every scheme carries a
/// secret (`password`/`token`) that must never render verbatim. Usernames are
/// not secret and are shown as-is to keep the output useful for diagnostics.
impl core::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Credentials::Basic { username, .. } => f
                .debug_struct("Credentials::Basic")
                .field("username", username)
                .field("password", &"***")
                .finish(),
            Credentials::Digest { username, .. } => f
                .debug_struct("Credentials::Digest")
                .field("username", username)
                .field("password", &"***")
                .finish(),
            Credentials::Bearer { .. } => f
                .debug_struct("Credentials::Bearer")
                .field("token", &"***")
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Security-blocker regression (pre-release audit): `Credentials` must
    // never render a secret via `{:?}`. Must FAIL if `Debug` reverts to a
    // plain `#[derive(Debug)]`.
    #[test]
    fn basic_debug_redacts_password_but_keeps_username() {
        let creds = Credentials::Basic {
            username: "admin".to_string(),
            password: "s3cr3t-password".to_string(),
        };
        let debug = format!("{creds:?}");
        assert!(!debug.contains("s3cr3t-password"), "leaked: {debug}");
        assert!(
            debug.contains("admin"),
            "username should be visible: {debug}"
        );
        assert!(debug.contains("***"), "expected redaction marker: {debug}");
    }

    #[test]
    fn digest_debug_redacts_password_but_keeps_username() {
        let creds = Credentials::Digest {
            username: "camera-user".to_string(),
            password: "hunter2-super-secret".to_string(),
        };
        let debug = format!("{creds:?}");
        assert!(!debug.contains("hunter2-super-secret"), "leaked: {debug}");
        assert!(
            debug.contains("camera-user"),
            "username should be visible: {debug}"
        );
        assert!(debug.contains("***"), "expected redaction marker: {debug}");
    }

    #[test]
    fn bearer_debug_redacts_token() {
        let creds = Credentials::bearer("super-secret-bearer-token-xyz");
        let debug = format!("{creds:?}");
        assert!(
            !debug.contains("super-secret-bearer-token-xyz"),
            "leaked: {debug}"
        );
        assert!(debug.contains("***"), "expected redaction marker: {debug}");
    }
}
