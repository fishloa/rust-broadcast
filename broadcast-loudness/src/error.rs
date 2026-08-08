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

    /// A sample rate of zero was requested.
    ///
    /// A loudness meter needs a positive sample rate to derive K‑weighting
    /// filter coefficients and to convert sample counts to seconds.
    #[error("sample rate must be greater than 0, got {got} Hz")]
    InvalidSampleRate {
        /// The invalid sample rate requested.
        got: u32,
    },

    /// A non-finite sample (NaN or ±Infinity) was passed to the meter.
    ///
    /// Non-finite values are rejected because they propagate through
    /// the IIR filter state and permanently poison all subsequent
    /// readings. The caller should either skip, replace, or clamp
    /// such samples before feeding them.
    #[error("non-finite sample at index {index} (channel {channel}): {value}")]
    NonFiniteSample {
        /// The index of the non-finite sample in the push call.
        index: usize,
        /// The channel index (zero-based) of the non-finite sample.
        channel: usize,
        /// The non-finite value received.
        value: f64,
    },
}

/// Result type for loudness operations.
pub type Result<T> = core::result::Result<T, Error>;
