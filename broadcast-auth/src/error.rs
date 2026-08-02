//! Error type for the shared auth layer.

/// Result alias for the crate's fallible operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors produced by challenge parsing / response computation.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `WWW-Authenticate` challenge value could not be parsed (RFC 7235 /
    /// RFC 2326 §14 for a Basic or Digest challenge).
    #[error("failed to parse challenge: {0}")]
    ChallengeParse(String),

    /// The `Authorization` response value could not be computed from the
    /// parsed challenge and credentials (e.g. an unsupported Digest
    /// `algorithm`/`qop`).
    #[error("failed to compute Authorization response: {0}")]
    ResponseCompute(String),

    /// [`crate::SignedUrlKeySet::new`] rejected a key whose secret is shorter
    /// than the required minimum (issue #747) — a setup-time error, never
    /// returned from request verification. The `kid` is included since this
    /// is a config-diagnostic error, not an attacker-facing one.
    #[error("signed-url key {kid:?} has a {actual}-byte secret, must be at least {min} bytes")]
    SignedUrlKeyTooShort {
        /// The offending key's id.
        kid: String,
        /// The required minimum secret length in bytes.
        min: usize,
        /// The offending secret's actual length in bytes.
        actual: usize,
    },

    /// [`crate::SignedUrlKeySet::sign`] was asked to mint a token for a `kid`
    /// the keyset has no secret for. Only reachable from the *signing* helper
    /// (used by tests/operators minting URLs) — never from
    /// [`crate::Verifier::verify`], which folds every signed-URL rejection
    /// reason into the same [`crate::AuthResult::Unauthorized`] (see that
    /// module's docs).
    #[error("signed-url keyset has no key with kid {0:?}")]
    UnknownSignedUrlKeyId(String),
}
