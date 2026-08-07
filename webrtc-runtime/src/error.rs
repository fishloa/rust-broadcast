//! Error types for WHIP/WHEP signalling.

/// Errors produced during WHIP/WHEP session signalling.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("missing required header: {header}")]
    MissingHeader { header: &'static str },

    #[error("invalid SDP: {reason}")]
    InvalidSdp { reason: alloc::string::String },

    #[error("invalid SDP fragment: {reason}")]
    InvalidSdpFragment { reason: alloc::string::String },

    #[error("wrong session state for {operation}: currently {state}")]
    WrongState {
        operation: &'static str,
        state: &'static str,
    },

    #[error("ETag mismatch: expected {expected:?}, got {got:?}")]
    ETagMismatch {
        expected: alloc::string::String,
        got: alloc::string::String,
    },

    #[error("counter-offer expired")]
    CounterOfferExpired,

    #[error("no active publisher")]
    NoPublisher,

    #[error("HTTP error: {status}")]
    Http { status: u16 },
}
