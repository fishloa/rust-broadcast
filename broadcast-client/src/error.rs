//! Error type for [`crate::Session`].

use thiserror::Error;

/// Errors returned by [`crate::Session::open`].
#[non_exhaustive]
#[derive(Debug, Error, Clone, PartialEq, Eq)]
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

/// Why a connection ended, as the host reports it to
/// [`crate::Session::on_closed`].
///
/// Typed rather than a `String` so the reconnect policy and the host's metrics
/// can tell a TLS failure from a reset without matching on text. The session
/// itself does not branch on the kind (any error is a close); a host can.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The peer or the network refused the connection (ECONNREFUSED, a proxy 502).
    Refused,
    /// The peer reset or aborted the connection mid-stream.
    Reset,
    /// A read or connect deadline expired.
    TimedOut,
    /// Name resolution failed.
    Dns {
        /// The host name that did not resolve.
        host: String,
    },
    /// The TLS handshake or the TLS layer failed.
    Tls {
        /// What the TLS stack reported.
        reason: String,
    },
    /// The peer closed cleanly, before the body or stream was complete.
    Eof,
    /// Anything the host cannot classify better, kept as its own description.
    Other {
        /// The host's description.
        reason: String,
    },
}

impl TransportError {
    /// Stable label, for a metric or a log field.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::Reset => "reset",
            Self::TimedOut => "timed out",
            Self::Dns { .. } => "dns",
            Self::Tls { .. } => "tls",
            Self::Eof => "eof",
            Self::Other { .. } => "other",
        }
    }

    /// The host's description of the failure, when it carried one.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Dns { host } => Some(host),
            Self::Tls { reason } | Self::Other { reason } => Some(reason),
            Self::Refused | Self::Reset | Self::TimedOut | Self::Eof => None,
        }
    }
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())?;
        if let Some(reason) = self.reason() {
            write!(f, ": {reason}")?;
        }
        Ok(())
    }
}

impl core::error::Error for TransportError {}

impl From<&str> for TransportError {
    fn from(reason: &str) -> Self {
        Self::Other {
            reason: reason.to_string(),
        }
    }
}

impl From<String> for TransportError {
    fn from(reason: String) -> Self {
        Self::Other { reason }
    }
}

#[cfg(test)]
mod tests {
    use super::TransportError;

    #[test]
    fn a_transport_error_carries_its_kind_without_text_matching() {
        // each kind is its own value, so a host branches on the variant
        assert_eq!(TransportError::Refused.name(), "refused");
        assert_eq!(TransportError::Reset.name(), "reset");
        assert_eq!(TransportError::TimedOut.name(), "timed out");
        assert_eq!(TransportError::Eof.name(), "eof");
        assert_eq!(
            TransportError::Dns {
                host: "cam.test".into()
            }
            .name(),
            "dns"
        );
        assert_ne!(
            TransportError::Tls {
                reason: "bad cert".into()
            },
            TransportError::Reset,
            "a TLS failure is not a reset"
        );
        // the display names the kind, and the reason only when there is one
        assert_eq!(TransportError::Reset.to_string(), "reset");
        assert_eq!(
            TransportError::Tls {
                reason: "bad cert".into()
            }
            .to_string(),
            "tls: bad cert"
        );
        assert_eq!(TransportError::Refused.reason(), None);
        assert_eq!(
            TransportError::from("connection reset by peer"),
            TransportError::Other {
                reason: "connection reset by peer".into()
            }
        );
    }
}
