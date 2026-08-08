//! Error types for WHIP/WHEP signalling.

/// Errors produced during WHIP/WHEP session signalling.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A response or request was missing a header required by the spec.
    #[error("missing required header: {header}")]
    MissingHeader {
        /// Name of the missing header.
        header: &'static str,
    },

    /// A Trickle ICE SDP fragment (`application/trickle-ice-sdpfrag`) could not be parsed.
    #[error("invalid SDP fragment: {reason}")]
    InvalidSdpFragment {
        /// Human-readable description of why the fragment was rejected.
        reason: alloc::string::String,
    },

    /// An operation was attempted while the session state machine was in an
    /// incompatible state.
    #[error("wrong session state for {operation}: currently {state}")]
    WrongState {
        /// Name of the operation that was attempted.
        operation: &'static str,
        /// Name of the state the session was actually in.
        state: &'static str,
    },

    /// The `ETag` supplied on a request did not match the session's current
    /// resource `ETag`.
    #[error("ETag mismatch: expected {expected:?}, got {got:?}")]
    ETagMismatch {
        /// The `ETag` the session expected.
        expected: alloc::string::String,
        /// The `ETag` actually supplied.
        got: alloc::string::String,
    },

    /// A WHEP client requested playback but no publisher is active for the resource.
    #[error("no active publisher")]
    NoPublisher,

    /// The remote peer returned an unexpected HTTP status code.
    #[error("HTTP error: {status}")]
    Http {
        /// The HTTP status code returned.
        status: u16,
    },

    /// An ICE, DTLS, or SRTP media-transport operation failed (feature
    /// `media`, see [`crate::media`]).
    ///
    /// Carries a formatted message rather than a `rtc-ice`/`rtc-dtls`/
    /// `rtc-srtp` error type so the default (non-`media`) build of this
    /// crate never has to know those dependency types exist.
    #[cfg(feature = "media")]
    #[error("media transport error: {0}")]
    Media(alloc::string::String),
}
