//! Error type for the shared auth layer.

/// Result alias for the crate's fallible operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors produced by challenge parsing / response computation.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    /// The OS random source failed while generating a Digest nonce-signing secret; no verifier is
    /// built, since a predictable secret would let nonces be forged.
    #[error("the OS random source is unavailable")]
    RandomSource,

    /// The `WWW-Authenticate` challenge value could not be parsed (RFC 7235 /
    /// RFC 2326 §14 for a Basic or Digest challenge).
    #[error("failed to parse challenge: {0}")]
    ChallengeParse(#[source] HttpAuthError),

    /// The `Authorization` response value could not be computed from the
    /// parsed challenge and credentials (e.g. an unsupported Digest
    /// `algorithm`/`qop`).
    #[error("failed to compute Authorization response: {0}")]
    ResponseCompute(#[source] HttpAuthError),

    /// A trusted-proxy entry was not an IP address or CIDR block (see
    /// [`crate::TrustedProxy`]) — a setup-time error.
    #[error("invalid trusted proxy {input:?}: {kind}")]
    InvalidTrustedProxy {
        /// The offending input.
        input: String,
        /// Why it is not a trusted proxy.
        kind: TrustedProxyFailure,
    },

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
    #[error("signed-url keyset has no key with kid {kid:?}")]
    UnknownSignedUrlKeyId {
        /// The key id the keyset has no secret for.
        kid: String,
    },

    /// A parameter that RFC 6750 (or `resource_metadata`) says MUST NOT repeat
    /// appeared twice in one challenge.
    #[error("duplicate parameter {name:?}")]
    DuplicateParameter {
        /// The offending parameter name (canonical, lower-cased).
        name: String,
    },

    /// `resource_metadata` was not an absolute `https` URL (RFC 9728 §3).
    #[error("invalid resource_metadata URL")]
    InvalidResourceMetadata,

    /// More challenges than [`crate::challenge::MAX_CHALLENGES`] were supplied.
    #[error("more than {max} challenges")]
    TooManyChallenges {
        /// The cap.
        max: usize,
    },

    /// A Bearer token cannot be carried in an `Authorization` header value —
    /// it contains a byte outside visible ASCII (control character, CR/LF, or
    /// non-ASCII) — so no header was produced rather than a corrupt or
    /// injectable one.
    #[error("bearer token is not a valid Authorization header value")]
    InvalidBearerToken,

    /// A URL's userinfo is not valid UTF-8 once percent-decoded. There is a
    /// single failure mode (the decoder refused the bytes), so no reason is
    /// carried; never expose the (encoded) credential.
    #[error("invalid percent-encoded userinfo")]
    InvalidUserinfo,

    /// A URL cannot carry userinfo (a cannot-be-a-base URL), so the named
    /// component (`"username"` / `"password"`) could not be removed.
    #[error("failed to strip {0} from URL")]
    UserinfoNotRemovable(&'static str),
}

/// Underlying error returned by the `http-auth` crate (a diagnostic string).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct HttpAuthError(pub String);

/// Why a trusted-proxy entry is not an IP address or CIDR block.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TrustedProxyFailure {
    /// The address is not a valid IPv4/IPv6 literal.
    #[error("not an IP address")]
    NotIpAddress,
    /// The prefix length is out of range for the address family.
    #[error("prefix length out of range")]
    PrefixOutOfRange,
}
