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
    /// The operation needs an HLS session.
    #[error("operation requires an HLS session")]
    NotHls,
    /// The variant index does not exist (or no master playlist was seen yet).
    #[error("no such variant: {0}")]
    NoSuchVariant(usize),
    /// A custom header name is owned by the session and cannot be set by the caller.
    #[error("reserved header: {name}")]
    ReservedHeader {
        /// The offending header name.
        name: String,
    },
    /// A custom header or initial cookie is not a valid HTTP field name or value.
    #[error("invalid header: {name}")]
    InvalidHeader {
        /// The offending header (or cookie) name.
        name: String,
    },
}

/// Transport error passed to [`crate::Session::on_closed`].
pub type TransportError = String;
