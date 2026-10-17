//! Configuration, units and the single source of defaults.
//!
//! Every default is a named constant in a `DEFAULT` block and `Default` is
//! implemented from it, so a C wrapper asks the crate (`lb_config_default`)
//! instead of duplicating numbers. Each default carries its evidence tag in
//! `docs/live-buffer/DESIGN.md` section 4 (`[tuned]` / `[sourced]` /
//! `[guess]`); all of them are **implementation-defined** (no standard
//! specifies a de-jitter policy).

use crate::error::{ConfigField, Error};
use broadcast_common::clock33::PCR_HZ;

/// Microseconds (arrival clock, intervals, targets).
pub type Micros = u64;
/// 27 MHz ticks (media positions: PCR domain for the estimator, PTS x 300 for
/// the playback/lead domain).
pub type Ticks27 = u64;

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
    pub min_window_us: Micros,
    /// Per-interval maximum resampling period (500 ms).
    pub interval_us: Micros,
    /// Interval maxima above this feed ONLY the max-hold, never the histogram (500 ms).
    pub outlier_us: Micros,
    /// Histogram bucket width (10 ms).
    pub bucket_us: u32,
    /// Histogram bucket count (400 => 0..4 s).
    pub buckets: u16,
    /// Per-interval forgetting factor, Q15 (0.98 => 32113).
    pub forget_q15: u16,
    /// Baseline quantile in permille (990).
    pub quantile_permille: u16,
    /// Stall-memory hold before decay (60 s).
    pub hold_us: Micros,
    /// Linear decay duration after the hold (60 s).
    pub decay_us: Micros,
    /// Hard cap of the sliding-minimum deque (4096).
    pub max_samples: u16,
    /// PCR jump threshold (1 s): media ahead of arrival by more, or backwards by more.
    pub discontinuity_us: Micros,
}

impl JitterConfig {
    /// The documented defaults.
    pub const DEFAULT: Self = Self {
        min_window_us: 2_000_000,
        interval_us: 500_000,
        outlier_us: 500_000,
        bucket_us: 10_000,
        buckets: 400,
        forget_q15: 32113,
        quantile_permille: 990,
        hold_us: 60_000_000,
        decay_us: 60_000_000,
        max_samples: 4096,
        discontinuity_us: 1_000_000,
    };

    /// Check every field.
    ///
    /// # Errors
    /// [`Error::ZeroLengthWindow`] or [`Error::InvalidConfig`] naming the field.
    pub fn validate(&self) -> Result<(), Error> {
        if self.min_window_us == 0 {
            return Err(Error::ZeroLengthWindow);
        }
        if self.interval_us == 0 {
            return Err(Error::InvalidConfig(ConfigField::Interval));
        }
        if self.min_window_us < self.interval_us.saturating_mul(2) {
            return Err(Error::InvalidConfig(ConfigField::MinWindow));
        }
        if self.bucket_us == 0 || self.buckets == 0 {
            return Err(Error::InvalidConfig(ConfigField::Bucket));
        }
        if u64::from(self.forget_q15) >= Q15_ONE {
            return Err(Error::InvalidConfig(ConfigField::Forget));
        }
        if self.quantile_permille == 0 || u64::from(self.quantile_permille) > PERMILLE {
            return Err(Error::InvalidConfig(ConfigField::Quantile));
        }
        if self.outlier_us == 0 {
            return Err(Error::InvalidConfig(ConfigField::Outlier));
        }
        if self.decay_us == 0 {
            return Err(Error::InvalidConfig(ConfigField::Decay));
        }
        if self.max_samples < MIN_SAMPLES_CAP {
            return Err(Error::InvalidConfig(ConfigField::Capacity));
        }
        if self.discontinuity_us == 0 {
            return Err(Error::InvalidConfig(ConfigField::Discontinuity));
        }
        Ok(())
    }

    /// Histogram range in microseconds (`buckets * bucket_us`).
    #[must_use]
    pub fn range_us(&self) -> u64 {
        u64::from(self.buckets) * u64::from(self.bucket_us)
    }
}

impl Default for JitterConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Target-buffer policy configuration.
///
/// `max_us` is a **hard cap**: neither the target, nor the start threshold,
/// nor the resume threshold ever exceeds it, so a caller can size its
/// compressed queue from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct PolicyConfig {
    /// Floor of every target (300 ms).
    pub min_us: Micros,
    /// Cold-start target without stall memory (1.8 s, owner-approved 2026-10-17).
    pub initial_us: Micros,
    /// Hard ceiling of every target and threshold (3 s).
    pub max_us: Micros,
    /// Baseline multiplier, Q8 (2.0 => 512).
    pub k1_q8: u16,
    /// Additive margin (100 ms).
    pub margin_us: Micros,
    /// Stall-peak multiplier, Q8 (1.2 => 307).
    pub k2_q8: u16,
    /// Resume target after a stall = `max(target, x * stall_peak)`, Q8 (1.5 => 384).
    pub after_rebuffer_q8: u16,
    /// Target decrease slew (100 ms per second).
    pub max_step_down_us_per_s: u32,
    /// Voluntary re-prime below this lead while playing (0 = disabled).
    pub reprime_floor_us: Micros,
    /// How long below the floor before re-priming (2 s).
    pub reprime_hold_us: Micros,
}

impl PolicyConfig {
    /// The documented defaults.
    pub const DEFAULT: Self = Self {
        min_us: 300_000,
        initial_us: 1_800_000,
        max_us: 3_000_000,
        k1_q8: 512,
        margin_us: 100_000,
        k2_q8: 307,
        after_rebuffer_q8: 384,
        max_step_down_us_per_s: 100_000,
        reprime_floor_us: 0,
        reprime_hold_us: 2_000_000,
    };

    /// Check every field.
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] naming the field.
    pub fn validate(&self) -> Result<(), Error> {
        if self.min_us == 0 || self.min_us > self.initial_us || self.initial_us > self.max_us {
            return Err(Error::InvalidConfig(ConfigField::Order));
        }
        if self.k1_q8 == 0 || self.k2_q8 == 0 || self.after_rebuffer_q8 == 0 {
            return Err(Error::InvalidConfig(ConfigField::Multiplier));
        }
        if self.max_step_down_us_per_s == 0 {
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
    pub starve_threshold_us: Micros,
    /// No chunk for this long => pause reason `NoData` (200 ms).
    pub nodata_after_us: Micros,
}

impl ControllerConfig {
    /// The documented defaults.
    pub const DEFAULT: Self = Self {
        jitter: JitterConfig::DEFAULT,
        policy: PolicyConfig::DEFAULT,
        starve_threshold_us: 20_000,
        nodata_after_us: 200_000,
    };

    /// Build and validate a configuration.
    ///
    /// # Errors
    /// See [`ControllerConfig::validate`].
    pub fn new(
        jitter: JitterConfig,
        policy: PolicyConfig,
        starve_threshold_us: Micros,
        nodata_after_us: Micros,
    ) -> Result<Self, Error> {
        let c = Self {
            jitter,
            policy,
            starve_threshold_us,
            nodata_after_us,
        };
        c.validate()?;
        Ok(c)
    }

    /// Validate the parts and the cross-field rules (`max_us` within the
    /// histogram range, `starve_threshold_us < min_us`).
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] / [`Error::ZeroLengthWindow`].
    pub fn validate(&self) -> Result<(), Error> {
        self.jitter.validate()?;
        self.policy.validate()?;
        if self.policy.max_us > self.jitter.range_us() {
            return Err(Error::InvalidConfig(ConfigField::Bucket));
        }
        if self.starve_threshold_us >= self.policy.min_us || self.nodata_after_us == 0 {
            return Err(Error::InvalidConfig(ConfigField::Threshold));
        }
        Ok(())
    }
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}
