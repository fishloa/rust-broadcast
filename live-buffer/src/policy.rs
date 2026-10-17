//! Target-buffer policy: need, clamp, asymmetric slew-limited follower and
//! the resume threshold. All integer (Q8 multipliers, microseconds).
//!
//! `max_us` is a hard cap: [`Policy::target_us`], [`Policy::start_us`] and
//! [`Policy::resume_us`] are never above it, whatever the estimator reports.

use crate::config::{Micros, PolicyConfig, Q8_SHIFT, US_PER_S, mul_q};

/// Target follower and threshold calculator.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    cfg: PolicyConfig,
    target: Micros,
    last_t: Option<u64>,
}

impl Policy {
    /// New policy at the floor target.
    #[must_use]
    pub fn new(cfg: PolicyConfig) -> Self {
        Self {
            cfg,
            target: cfg.min_us,
            last_t: None,
        }
    }

    /// Unclamped need: `max(k1 * baseline + margin, k2 * stall_peak)`.
    #[must_use]
    pub fn need_us(&self, baseline_us: Micros, stall_peak_us: Micros) -> Micros {
        let a = mul_q(baseline_us, self.cfg.k1_q8, Q8_SHIFT).saturating_add(self.cfg.margin_us);
        let b = mul_q(stall_peak_us, self.cfg.k2_q8, Q8_SHIFT);
        a.max(b)
    }

    /// Clamp into `[min_us, max_us]`.
    #[must_use]
    pub fn clamp(&self, v: Micros) -> Micros {
        v.clamp(self.cfg.min_us, self.cfg.max_us)
    }

    /// Asymmetric follower: rise instantly, fall at most
    /// `max_step_down_us_per_s`. Returns the new target.
    pub fn follow(&mut self, need_us: Micros, now_us: u64) -> Micros {
        let raw = self.clamp(need_us);
        let dt = self.last_t.map_or(0, |t| now_us.saturating_sub(t));
        let fall =
            u128::from(self.cfg.max_step_down_us_per_s) * u128::from(dt) / u128::from(US_PER_S);
        let fall = u64::try_from(fall).unwrap_or(u64::MAX);
        self.target = raw.max(self.target.saturating_sub(fall));
        self.last_t = Some(now_us);
        self.target
    }

    /// Raise (never lower) the target to at least `clamp(need)`.
    pub fn raise(&mut self, need_us: Micros) {
        self.target = self.target.max(self.clamp(need_us));
    }

    /// Back to the floor.
    pub fn reset(&mut self) {
        self.target = self.cfg.min_us;
        self.last_t = None;
    }

    /// Current target.
    #[must_use]
    pub fn target_us(&self) -> Micros {
        self.target
    }

    /// First-fill threshold: `max(initial, target)`.
    #[must_use]
    pub fn start_us(&self) -> Micros {
        self.clamp(self.cfg.initial_us.max(self.target))
    }

    /// Post-stall resume threshold: `clamp(max(target, after_rebuffer * stall_peak))`.
    #[must_use]
    pub fn resume_us(&self, stall_peak_us: Micros) -> Micros {
        self.clamp(
            self.target
                .max(mul_q(stall_peak_us, self.cfg.after_rebuffer_q8, Q8_SHIFT)),
        )
    }

    /// The configured cap.
    #[must_use]
    pub fn max_us(&self) -> Micros {
        self.cfg.max_us
    }
}
