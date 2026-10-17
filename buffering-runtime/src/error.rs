//! Typed errors. Nothing in this crate panics on input; invalid
//! configuration is the only failure and is reported by value.

/// Which configuration field failed validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ConfigField {
    /// `JitterConfig::min_window_us` shorter than two intervals.
    MinWindow,
    /// `JitterConfig::interval_us` is zero.
    Interval,
    /// `bucket_us` or `buckets` is zero, or `max_us` exceeds the histogram range.
    Bucket,
    /// `forget_q15` is not below 32768 (1.0 in Q15).
    Forget,
    /// `quantile_permille` is zero or above 1000.
    Quantile,
    /// `min_us <= initial_us <= max_us` does not hold (or `min_us` is zero).
    Order,
    /// `max_samples` below the minimum deque capacity.
    Capacity,
    /// `starve_threshold_us` not below `min_us`, or `nodata_after_us` zero.
    Threshold,
    /// A Q8 multiplier is zero.
    Multiplier,
    /// `outlier_us` is zero.
    Outlier,
    /// `decay_us` is zero.
    Decay,
    /// `discontinuity_us` is zero.
    Discontinuity,
    /// `max_step_down_us_per_s` is zero.
    Slew,
    /// `track_silence_timeout_us` is zero.
    TrackSilence,
    /// `granularity_decay_us` is zero.
    GranularityDecay,
}

impl ConfigField {
    /// Stable label for the field.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::MinWindow => "min_window",
            Self::Interval => "interval",
            Self::Bucket => "bucket",
            Self::Forget => "forget",
            Self::Quantile => "quantile",
            Self::Order => "order",
            Self::Capacity => "capacity",
            Self::Threshold => "threshold",
            Self::Multiplier => "multiplier",
            Self::Outlier => "outlier",
            Self::Decay => "decay",
            Self::Discontinuity => "discontinuity",
            Self::Slew => "slew",
            Self::TrackSilence => "track_silence",
            Self::GranularityDecay => "granularity_decay",
        }
    }
}
broadcast_common::impl_spec_display!(ConfigField);

/// Construction / validation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Error {
    /// A configuration field is out of range.
    InvalidConfig(ConfigField),
    /// `min_window_us` is zero (a zero-length sliding window has no minimum).
    ZeroLengthWindow,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidConfig(field) => write!(f, "invalid configuration: {field}"),
            Self::ZeroLengthWindow => f.write_str("zero-length minimum window"),
        }
    }
}

impl core::error::Error for Error {}
