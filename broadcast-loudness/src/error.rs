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

    /// The K-weighting filter coefficients are only specified for 48 kHz
    /// (ITU-R BS.1770-5 Annex 1 Tables 1–2). The spec states that other
    /// sample rates "require different coefficient values" but does not
    /// tabulate them or provide a formula.
    ///
    /// Resample the input to 48 kHz before measurement.
    #[error("K-weighting coefficients are only defined for 48 kHz, got {got} Hz (ITU-R BS.1770-5 Annex 1). Resample to 48 kHz.")]
    UnsupportedSampleRate {
        /// The unsupported sample rate requested.
        got: u32,
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
