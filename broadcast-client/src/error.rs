//! Error type for [`crate::Session`].

use thiserror::Error;

/// Errors returned by [`crate::Session::open`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum Error {
    /// The session URL uses a scheme that is not supported in BC-1.
    #[error("unsupported scheme: {0}")]
    UnsupportedScheme(String),
    /// The session URL could not be parsed.
    #[error("invalid url: {0}")]
    InvalidUrl(String),
}

/// Transport error passed to [`crate::Session::on_closed`].
pub type TransportError = String;
