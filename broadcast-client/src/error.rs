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
    /// The operation needs an adaptive-bitrate session (HLS or DASH): `select_variant` and
    /// `select_audio_rendition` act on a master playlist or an MPD's Representations, which
    /// TS-over-HTTP and RTSP sessions do not have.
    #[error("operation requires an HLS or DASH session")]
    NotHls,
    /// The variant index does not exist (or no master playlist was seen yet).
    #[error("no such variant: {0}")]
    NoSuchVariant(usize),
    /// The audio rendition index does not exist for the chosen variant (or no master playlist
    /// was seen yet).
    #[error("no such audio rendition")]
    NoSuchRendition,
    /// A custom header name is owned by the session and cannot be set by the caller.
    #[error("reserved header: {name}")]
    ReservedHeader {
        /// The offending header name.
        name: String,
    },
    /// The credentials cannot be sent as an HTTP header value (control characters in the token,
    /// user name or password).
    #[error("credentials contain characters that cannot be sent in a header")]
    InvalidCredentials,
    /// A custom header or initial cookie is not a valid HTTP field name or value.
    #[error("invalid header: {name}")]
    InvalidHeader {
        /// The offending header (or cookie) name.
        name: String,
    },
    /// A [`crate::Config`] field holds a value no session can run with.
    #[error("invalid config: {field}")]
    InvalidConfig {
        /// The offending field name.
        field: &'static str,
    },
}

/// Transport error passed to [`crate::Session::on_closed`].
pub type TransportError = String;
