//! Configuration, units and the single source of defaults.
//!
//! Every default is a named constant in a `DEFAULT` block and `Default` is
//! implemented from it, so a C wrapper asks the crate (`br_config_default`)
//! instead of duplicating numbers. Each default carries its evidence tag in
//! `docs/buffering-runtime/DESIGN.md` section 4 (`[tuned]` / `[sourced]` /
//! `[guess]`); all of them are **implementation-defined** (no standard
//! specifies a de-jitter policy).
//!
//! Every span is a [`Duration`] (see [`crate::time`]); the controller keeps
//! them in whole microseconds internally.

use core::time::Duration;

use broadcast_common::clock33::PCR_HZ;

use crate::error::{ConfigField, Error};
use crate::time::{duration_micros, micros_as_duration};

/// Microseconds per second.
pub const US_PER_S: u64 = 1_000_000;
/// 27 MHz ticks per microsecond.
pub(crate) const TICKS_PER_US: u64 = PCR_HZ / US_PER_S;
/// Q15 fixed point: shift, and the value of 1.0.
pub(crate) const Q15_SHIFT: u32 = 15;
pub(crate) const Q15_ONE: u64 = 1 << Q15_SHIFT;
/// Q8 fixed point shift.
pub(crate) const Q8_SHIFT: u32 = 8;
/// Quantiles are expressed in permille.
pub(crate) const PERMILLE: u64 = 1000;
/// Smallest legal `max_samples`.
pub(crate) const MIN_SAMPLES_CAP: u16 = 8;
/// Over-max re-arm fraction: re-armed when `need <= max * NUM / DEN`.
pub(crate) const REARM_NUM: u64 = 3;
pub(crate) const REARM_DEN: u64 = 4;

/// `value * q >> shift`, widened so it cannot overflow, saturating at `u64::MAX`.
pub(crate) fn mul_q(value: u64, q: u16, shift: u32) -> u64 {
    let wide = (u128::from(value) * u128::from(q)) >> shift;
    u64::try_from(wide).unwrap_or(u64::MAX)
}

/// Delivery-jitter estimator configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct JitterConfig {
    /// Sliding-minimum window for the relative transit `D` (2 s).
    pub min_window: Duration,
    /// Per-interval maximum resampling period (500 ms).
    pub interval: Duration,
    /// Interval maxima above this feed ONLY the max-hold, never the histogram (500 ms).
    pub outlier: Duration,
    /// Histogram bucket width (10 ms).
    pub bucket: Duration,
    /// Histogram bucket count (400 => 0..4 s).
    pub buckets: u16,
    /// Per-interval forgetting factor, Q15 (0.98 => 32113).
    pub forget_q15: u16,
    /// Baseline quantile in permille (990).
    pub quantile_permille: u16,
    /// Stall-memory hold before decay (60 s).
    pub hold: Duration,
    /// Linear decay duration after the hold (60 s).
    pub decay: Duration,
    /// Hard cap of the sliding-minimum deque (4096).
    pub max_samples: u16,
    /// PCR jump threshold (1 s): media ahead of arrival by more, or backwards by more.
    pub discontinuity: Duration,
}

impl JitterConfig {
    /// The documented defaults.
    pub const DEFAULT: Self = Self {
        min_window: Duration::from_secs(2),
        interval: Duration::from_millis(500),
        outlier: Duration::from_millis(500),
        bucket: Duration::from_millis(10),
        buckets: 400,
        forget_q15: 32113,
        quantile_permille: 990,
        hold: Duration::from_secs(60),
        decay: Duration::from_secs(60),
        max_samples: 4096,
        discontinuity: Duration::from_secs(1),
    };

    /// Check every field.
    ///
    /// # Errors
    /// [`Error::ZeroLengthWindow`] or [`Error::InvalidConfig`] naming the field.
    pub fn validate(&self) -> Result<(), Error> {
        if self.min_window.is_zero() {
            return Err(Error::ZeroLengthWindow);
        }
        if self.interval.is_zero() {
            return Err(Error::InvalidConfig(ConfigField::Interval));
        }
        if duration_micros(self.min_window) < duration_micros(self.interval).saturating_mul(2) {
            return Err(Error::InvalidConfig(ConfigField::MinWindow));
        }
        if self.bucket.is_zero() || self.buckets == 0 {
            return Err(Error::InvalidConfig(ConfigField::Bucket));
        }
        if u64::from(self.forget_q15) >= Q15_ONE {
            return Err(Error::InvalidConfig(ConfigField::Forget));
        }
        if self.quantile_permille == 0 || u64::from(self.quantile_permille) > PERMILLE {
            return Err(Error::InvalidConfig(ConfigField::Quantile));
        }
        if self.outlier.is_zero() {
            return Err(Error::InvalidConfig(ConfigField::Outlier));
        }
        if self.decay.is_zero() {
            return Err(Error::InvalidConfig(ConfigField::Decay));
        }
        if self.max_samples < MIN_SAMPLES_CAP {
            return Err(Error::InvalidConfig(ConfigField::Capacity));
        }
        if self.discontinuity.is_zero() {
            return Err(Error::InvalidConfig(ConfigField::Discontinuity));
        }
        Ok(())
    }

    /// Histogram range (`buckets * bucket`).
    #[must_use]
    pub fn range(&self) -> Duration {
        micros_as_duration(self.range_us())
    }

    pub(crate) fn range_us(&self) -> u64 {
        u64::from(self.buckets) * duration_micros(self.bucket)
    }
}

impl Default for JitterConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Target-buffer policy configuration.
///
/// `max` is a **hard cap**: neither the target, nor the start threshold, nor
/// the resume threshold ever exceeds it, so a caller can size its compressed
/// queue from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct PolicyConfig {
    /// Floor of every target (300 ms).
    pub min: Duration,
    /// Cold-start target without stall memory (1.8 s, owner-approved 2026-10-17).
    pub initial: Duration,
    /// Hard ceiling of every target and threshold (3 s).
    pub max: Duration,
    /// Baseline multiplier, Q8 (2.0 => 512).
    pub k1_q8: u16,
    /// Additive margin (100 ms).
    pub margin: Duration,
    /// Stall-peak multiplier, Q8 (1.2 => 307).
    pub k2_q8: u16,
    /// Resume target after a stall = `max(target, x * stall_peak)`, Q8 (1.25 => 320).
    pub after_rebuffer_q8: u16,
    /// Target decrease rate per second of clock (100 ms per second).
    pub max_step_down_per_s: Duration,
    /// Voluntary re-prime below this lead while playing (0 = disabled).
    pub reprime_floor: Duration,
    /// How long below the floor before re-priming (2 s).
    pub reprime_hold: Duration,
}

impl PolicyConfig {
    /// The documented defaults.
    pub const DEFAULT: Self = Self {
        min: Duration::from_millis(300),
        initial: Duration::from_millis(1_800),
        max: Duration::from_secs(3),
        k1_q8: 512,
        margin: Duration::from_millis(100),
        k2_q8: 307,
        after_rebuffer_q8: 320,
        max_step_down_per_s: Duration::from_millis(100),
        reprime_floor: Duration::ZERO,
        reprime_hold: Duration::from_secs(2),
    };

    /// Check every field.
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] naming the field.
    pub fn validate(&self) -> Result<(), Error> {
        if self.min.is_zero() || self.min > self.initial || self.initial > self.max {
            return Err(Error::InvalidConfig(ConfigField::Order));
        }
        if self.k1_q8 == 0 || self.k2_q8 == 0 || self.after_rebuffer_q8 == 0 {
            return Err(Error::InvalidConfig(ConfigField::Multiplier));
        }
        if self.max_step_down_per_s.is_zero() {
            return Err(Error::InvalidConfig(ConfigField::Slew));
        }
        Ok(())
    }
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Whole-controller configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ControllerConfig {
    /// Estimator configuration.
    pub jitter: JitterConfig,
    /// Policy configuration.
    pub policy: PolicyConfig,
    /// Starved when lead is at or below this while the clock advances (20 ms).
    pub starve_threshold: Duration,
    /// No chunk for this long => pause reason `NoData` (200 ms).
    pub nodata_after: Duration,
    /// A track with no delivery for this long is excluded from the effective
    /// media end (`on_track_media` / `register_track`), so a dead track cannot
    /// starve playback forever (3 s).
    #[cfg_attr(feature = "serde", serde(default = "default_track_silence_timeout"))]
    pub track_silence_timeout: Duration,
    /// Per-track delivery-granularity floor on/off (default on): the target is
    /// kept at least `max over tracks of (largest media-end step) + policy.margin`,
    /// so a track that delivers in coarse bursts (MP2 audio in ~0.3 s bursts)
    /// cannot starve between bursts. See `DESIGN.md` section 0.7.
    #[cfg_attr(feature = "serde", serde(default = "default_track_granularity"))]
    pub track_granularity: bool,
    /// How long a track's largest delivery step is held before it decays (30 s).
    #[cfg_attr(feature = "serde", serde(default = "default_granularity_hold"))]
    pub granularity_hold: Duration,
    /// Linear decay of a held delivery step to zero after the hold (30 s).
    #[cfg_attr(feature = "serde", serde(default = "default_granularity_decay"))]
    pub granularity_decay: Duration,
}

/// Default of [`ControllerConfig::track_silence_timeout`].
pub const DEFAULT_TRACK_SILENCE_TIMEOUT: Duration = Duration::from_secs(3);
/// Default of [`ControllerConfig::track_granularity`].
pub const DEFAULT_TRACK_GRANULARITY: bool = true;
/// Default of [`ControllerConfig::granularity_hold`].
pub const DEFAULT_GRANULARITY_HOLD: Duration = Duration::from_secs(30);
/// Default of [`ControllerConfig::granularity_decay`].
pub const DEFAULT_GRANULARITY_DECAY: Duration = Duration::from_secs(30);

#[cfg(feature = "serde")]
fn default_track_silence_timeout() -> Duration {
    DEFAULT_TRACK_SILENCE_TIMEOUT
}
#[cfg(feature = "serde")]
fn default_track_granularity() -> bool {
    DEFAULT_TRACK_GRANULARITY
}
#[cfg(feature = "serde")]
fn default_granularity_hold() -> Duration {
    DEFAULT_GRANULARITY_HOLD
}
#[cfg(feature = "serde")]
fn default_granularity_decay() -> Duration {
    DEFAULT_GRANULARITY_DECAY
}

impl ControllerConfig {
    /// The documented defaults.
    pub const DEFAULT: Self = Self {
        jitter: JitterConfig::DEFAULT,
        policy: PolicyConfig::DEFAULT,
        starve_threshold: Duration::from_millis(20),
        nodata_after: Duration::from_millis(200),
        track_silence_timeout: DEFAULT_TRACK_SILENCE_TIMEOUT,
        track_granularity: DEFAULT_TRACK_GRANULARITY,
        granularity_hold: DEFAULT_GRANULARITY_HOLD,
        granularity_decay: DEFAULT_GRANULARITY_DECAY,
    };

    /// Build and validate a configuration (the per-track silence timeout takes
    /// its default; change it with
    /// [`with_track_silence_timeout`](Self::with_track_silence_timeout)).
    ///
    /// # Errors
    /// See [`ControllerConfig::validate`].
    pub fn new(
        jitter: JitterConfig,
        policy: PolicyConfig,
        starve_threshold: Duration,
        nodata_after: Duration,
    ) -> Result<Self, Error> {
        let c = Self {
            jitter,
            policy,
            starve_threshold,
            nodata_after,
            track_silence_timeout: DEFAULT_TRACK_SILENCE_TIMEOUT,
            track_granularity: DEFAULT_TRACK_GRANULARITY,
            granularity_hold: DEFAULT_GRANULARITY_HOLD,
            granularity_decay: DEFAULT_GRANULARITY_DECAY,
        };
        c.validate()?;
        Ok(c)
    }

    /// Replace the per-track silence timeout and re-validate.
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] ([`ConfigField::TrackSilence`]) when zero.
    pub fn with_track_silence_timeout(mut self, timeout: Duration) -> Result<Self, Error> {
        self.track_silence_timeout = timeout;
        self.validate()?;
        Ok(self)
    }

    /// Switch the per-track delivery-granularity floor on or off.
    #[must_use]
    pub fn with_track_granularity(mut self, enabled: bool) -> Self {
        self.track_granularity = enabled;
        self
    }

    /// Replace the granularity memory (hold, then linear decay) and re-validate.
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] ([`ConfigField::GranularityDecay`]) when `decay` is zero.
    pub fn with_granularity_memory(
        mut self,
        hold: Duration,
        decay: Duration,
    ) -> Result<Self, Error> {
        self.granularity_hold = hold;
        self.granularity_decay = decay;
        self.validate()?;
        Ok(self)
    }

    /// Validate the parts and the cross-field rules (`max` within the
    /// histogram range, `starve_threshold < min`, non-zero silence timeout).
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] / [`Error::ZeroLengthWindow`].
    pub fn validate(&self) -> Result<(), Error> {
        self.jitter.validate()?;
        self.policy.validate()?;
        if duration_micros(self.policy.max) > self.jitter.range_us() {
            return Err(Error::InvalidConfig(ConfigField::Bucket));
        }
        if self.starve_threshold >= self.policy.min || self.nodata_after.is_zero() {
            return Err(Error::InvalidConfig(ConfigField::Threshold));
        }
        if self.track_silence_timeout.is_zero() {
            return Err(Error::InvalidConfig(ConfigField::TrackSilence));
        }
        if self.granularity_decay.is_zero() {
            return Err(Error::InvalidConfig(ConfigField::GranularityDecay));
        }
        Ok(())
    }
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::{ControllerConfig, Error, JitterConfig, PolicyConfig};
    use core::time::Duration;

    #[test]
    fn the_defaults_are_spans_and_validate() {
        ControllerConfig::DEFAULT
            .validate()
            .expect("defaults validate");
        assert_eq!(JitterConfig::DEFAULT.min_window, Duration::from_secs(2));
        assert_eq!(JitterConfig::DEFAULT.range(), Duration::from_secs(4));
        assert_eq!(PolicyConfig::DEFAULT.max, Duration::from_secs(3));
        assert_eq!(
            ControllerConfig::DEFAULT.nodata_after,
            Duration::from_millis(200)
        );
    }

    /// A zero-length window is the typed error: the span type has no "unset"
    /// distinguished from zero, so the validation that compared against the
    /// old zero newtype must still catch it.
    #[test]
    fn a_zero_length_window_is_rejected() {
        let jitter = JitterConfig {
            min_window: Duration::ZERO,
            ..JitterConfig::DEFAULT
        };
        assert_eq!(jitter.validate(), Err(Error::ZeroLengthWindow));
    }

    /// The spans carry the same magnitudes the microsecond fields did, so a
    /// migration that only changed the type cannot silently move a default.
    #[test]
    fn every_default_span_is_its_documented_size() {
        let c = ControllerConfig::DEFAULT;
        assert_eq!(c.starve_threshold, Duration::from_millis(20));
        assert_eq!(c.policy.initial, Duration::from_millis(1_800));
        assert_eq!(c.policy.margin, Duration::from_millis(100));
        assert_eq!(c.policy.max_step_down_per_s, Duration::from_millis(100));
        assert_eq!(c.jitter.bucket, Duration::from_millis(10));
        assert_eq!(c.granularity_hold, Duration::from_secs(30));
        assert_eq!(c.track_silence_timeout, Duration::from_secs(3));
    }
}
