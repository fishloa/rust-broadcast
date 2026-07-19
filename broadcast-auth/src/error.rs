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
}
