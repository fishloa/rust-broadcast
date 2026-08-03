use thiserror::Error;

/// Loudness measurement error.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// A channel index is out of range for the configured layout.
    #[error("channel index {index} out of range (layout has {layout} channels)")]
    ChannelOutOfRange {
        /// The requested channel index.
        index: usize,
        /// The channel count of the configured layout.
        layout: usize,
    },

    /// Push was called after `finish()`.
    #[error("meter has been finished, no more samples accepted")]
    Finished,

    /// Too few samples provided for the channel count.
    #[error("expected {expected} channels, got {got}")]
    ChannelMismatch {
        /// Number of channels expected.
        expected: usize,
        /// Number of channels provided.
        got: usize,
    },

    /// A feature is not yet implemented.
    #[error("{what} is not yet implemented")]
    NotImplemented {
        /// Description of the missing feature.
        what: &'static str,
    },
}

/// Result type for loudness operations.
pub type Result<T> = core::result::Result<T, Error>;
