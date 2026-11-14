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
    /// `bucket` or `buckets` is zero, or `max` exceeds the histogram range.
    Bucket,
    /// `forget_q15` is not below 32768 (1.0 in Q15).
    Forget,
    /// `quantile_permille` is zero or above 1000.
    Quantile,
    /// `min <= initial <= max` does not hold (or `min` is zero).
    Order,
    /// `max_samples` below the minimum deque capacity.
    Capacity,
    /// `starve_threshold` not below `min`, or `nodata_after` zero.
    Threshold,
    /// A Q8 multiplier is zero.
    Multiplier,
    /// `outlier` is zero.
    Outlier,
    /// `decay` is zero.
    Decay,
    /// `discontinuity` is zero.
    Discontinuity,
    /// `max_step_down_per_s` is zero.
    Slew,
    /// `track_silence_timeout` is zero.
    TrackSilence,
    /// `granularity_decay` is zero.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Error {
    /// A configuration field is out of range.
    #[error("invalid configuration: {0}")]
    InvalidConfig(ConfigField),
    /// `min_window` is zero (a zero-length sliding window has no minimum).
    #[error("zero-length minimum window")]
    ZeroLengthWindow,
}
