//! Target-buffer policy: need, clamp, asymmetric slew-limited follower and
//! the resume threshold. All integer (Q8 multipliers, microseconds), with
//! spans crossing the edge as [`Duration`].
//!
//! `max` is a hard cap: [`Policy::target`], [`Policy::start`] and
//! [`Policy::resume`] are never above it, whatever the estimator reports.

use core::time::Duration;

use crate::config::{PolicyConfig, Q8_SHIFT, US_PER_S, mul_q};
use crate::time::{duration_micros, micros_as_duration};

/// Target follower and threshold calculator.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    cfg: PolicyConfig,
    min_us: u64,
    initial_us: u64,
    max_us: u64,
    margin_us: u64,
    step_down_us_per_s: u64,
    reprime_floor_us: u64,
    reprime_hold_us: u64,
    target: u64,
    last_t: Option<u64>,
}

impl Policy {
    /// New policy at the floor target.
    #[must_use]
    pub fn new(cfg: PolicyConfig) -> Self {
        Self {
            cfg,
            min_us: duration_micros(cfg.min),
            initial_us: duration_micros(cfg.initial),
            max_us: duration_micros(cfg.max),
            margin_us: duration_micros(cfg.margin),
            step_down_us_per_s: duration_micros(cfg.max_step_down_per_s),
            reprime_floor_us: duration_micros(cfg.reprime_floor),
            reprime_hold_us: duration_micros(cfg.reprime_hold),
            target: duration_micros(cfg.min),
            last_t: None,
        }
    }

    /// Unclamped need:
    /// `max(k1 * baseline + margin, k2 * stall_peak, granularity + margin)`
    /// (the last term only when `granularity_us > 0`).
    #[must_use]
    pub fn need_with_granularity_us(
        &self,
        baseline_us: u64,
        stall_peak_us: u64,
        granularity_us: u64,
    ) -> u64 {
        let a = mul_q(baseline_us, self.cfg.k1_q8, Q8_SHIFT).saturating_add(self.margin_us);
        let b = mul_q(stall_peak_us, self.cfg.k2_q8, Q8_SHIFT);
        let c = self.granularity_need_us(granularity_us);
        a.max(b).max(c)
    }

    /// Unclamped need without a granularity floor:
    /// `max(k1 * baseline + margin, k2 * stall_peak)`.
    #[must_use]
    pub fn need_us(&self, baseline_us: u64, stall_peak_us: u64) -> u64 {
        self.need_with_granularity_us(baseline_us, stall_peak_us, 0)
    }

    /// The delivery-granularity floor: `granularity + margin`, 0 when there is
    /// no granularity to cover.
    #[must_use]
    pub fn granularity_need_us(&self, granularity_us: u64) -> u64 {
        if granularity_us == 0 {
            0
        } else {
            granularity_us.saturating_add(self.margin_us)
        }
    }

    /// Clamp into `[min, max]`.
    #[must_use]
    pub fn clamp(&self, v: u64) -> u64 {
        v.clamp(self.min_us, self.max_us)
    }

    /// Asymmetric follower: rise instantly, fall at most
    /// `max_step_down_per_s`. Returns the new target.
    pub fn follow(&mut self, need_us: u64, now_us: u64) -> u64 {
        let raw = self.clamp(need_us);
        let dt = self.last_t.map_or(0, |t| now_us.saturating_sub(t));
        let fall = u128::from(self.step_down_us_per_s) * u128::from(dt) / u128::from(US_PER_S);
        let fall = u64::try_from(fall).unwrap_or(u64::MAX);
        self.target = raw.max(self.target.saturating_sub(fall));
        self.last_t = Some(now_us);
        self.target
    }

    /// Raise (never lower) the target to at least `clamp(need)`.
    pub fn raise(&mut self, need_us: u64) {
        self.target = self.target.max(self.clamp(need_us));
    }

    /// Back to the floor.
    pub fn reset(&mut self) {
        self.target = self.min_us;
        self.last_t = None;
    }

    /// Current target.
    #[must_use]
    pub fn target(&self) -> Duration {
        micros_as_duration(self.target)
    }

    /// Current target in whole microseconds.
    pub(crate) fn target_us(&self) -> u64 {
        self.target
    }

    /// First-fill threshold: `max(initial, target)`.
    #[must_use]
    pub fn start(&self) -> Duration {
        micros_as_duration(self.clamp(self.initial_us.max(self.target)))
    }

    /// Post-stall resume threshold: `clamp(max(target, after_rebuffer * stall_peak))`.
    #[must_use]
    pub fn resume(&self, stall_peak: Duration) -> Duration {
        micros_as_duration(self.clamp(self.target.max(mul_q(
            duration_micros(stall_peak),
            self.cfg.after_rebuffer_q8,
            Q8_SHIFT,
        ))))
    }

    /// The configured cap.
    #[must_use]
    pub fn max(&self) -> Duration {
        self.cfg.max
    }

    /// The configured voluntary re-prime floor (`0` when disabled).
    #[must_use]
    pub fn reprime_floor(&self) -> Duration {
        micros_as_duration(self.reprime_floor_us)
    }

    /// The configured re-prime hold.
    #[must_use]
    pub fn reprime_hold(&self) -> Duration {
        micros_as_duration(self.reprime_hold_us)
    }
}
