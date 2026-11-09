use thiserror::Error;

/// Loudness measurement error.
#[derive(Debug, Error, Clone, PartialEq)]
#[non_exhaustive]
pub enum Error {
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

    /// A sample rate below [`crate::MIN_SAMPLE_RATE`] or above
    /// [`crate::MAX_SAMPLE_RATE`] was requested.
    ///
    /// Below the minimum the K‑weighting shelf corner is at or above Nyquist
    /// and the derived filter is unstable; the upper bound keeps the
    /// sliding-window allocation finite.
    #[error(
        "sample rate must be between {} and {} Hz, got {got} Hz",
        crate::MIN_SAMPLE_RATE,
        crate::MAX_SAMPLE_RATE
    )]
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

    /// A [`crate::ChannelLayout::Custom`] weight is non-finite or negative,
    /// or no weight is positive, so the layout measures nothing.
    #[error("invalid custom channel weights")]
    InvalidChannelWeights,

    /// A sample whose magnitude exceeds
    /// [`crate::LoudnessMeter::max_sample_magnitude`], which could carry a
    /// block above the +30 LKFS top of the meter's loudness histograms.
    #[error(
        "sample {value} at index {index} (channel {channel}) exceeds the measurable magnitude {max}"
    )]
    SampleTooLoud {
        /// The index of the sample in the push call.
        index: usize,
        /// The channel index (zero-based) of the sample.
        channel: usize,
        /// The value received.
        value: f64,
        /// The meter's largest accepted magnitude.
        max: f64,
    },
}

/// Result type for loudness operations.
pub type Result<T> = core::result::Result<T, Error>;
