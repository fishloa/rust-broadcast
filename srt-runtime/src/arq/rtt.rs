//! RTT / RTTVar EWMA estimation — `draft-sharabayko-srt-01` §4.10 (Round-Trip
//! Time Estimation), `specs/rules/srt-arq.md` rules 29-31.
//!
//! Both formulas and the initial values are quoted verbatim in
//! `specs/rules/srt-arq.md`:
//! - rule 29: `RTT = 7/8 * RTT + 1/8 * rtt` (L3009).
//! - rule 30: `RTTVar = 3/4 * RTTVar + 1/4 * abs(RTT - rtt)` (L3011-3013),
//!   where `RTT` on the right-hand side is the value *before* this update
//!   (both formulas are evaluated from the same pre-update state, per the
//!   draft's L3009/L3011 pairing).
//! - rule 31: RTT/RTTVar are in microseconds; the initial RTT is 100 ms, the
//!   initial RTTVar is 50 ms (L3017-3018).
//!
//! Used by both [`crate::arq::Sender`] (rule 33: updates from each Full
//! ACK's carried RTT sample) and [`crate::arq::Receiver`] (rules 26-28:
//! updates from each ACK-send / ACKACK-arrival round trip).

use core::time::Duration;

/// Initial RTT — 100 ms (`specs/rules/srt-arq.md` rule 31, L3017-3018).
pub const INITIAL_RTT: Duration = Duration::from_millis(100);
/// Initial RTTVar — 50 ms (`specs/rules/srt-arq.md` rule 31, L3017-3018).
pub const INITIAL_RTT_VAR: Duration = Duration::from_millis(50);

/// RTT/RTTVar estimator, updated by the rule-29/30 EWMA formulas on each new
/// round-trip sample (`draft-sharabayko-srt-01` §4.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RttEstimator {
    rtt: Duration,
    rtt_var: Duration,
}

impl Default for RttEstimator {
    fn default() -> Self {
        RttEstimator {
            rtt: INITIAL_RTT,
            rtt_var: INITIAL_RTT_VAR,
        }
    }
}

impl RttEstimator {
    /// A fresh estimator at the spec-mandated initial values (rule 31).
    pub fn new() -> Self {
        Self::default()
    }

    /// The current RTT estimate.
    pub fn rtt(&self) -> Duration {
        self.rtt
    }

    /// The current RTTVar estimate.
    pub fn rtt_var(&self) -> Duration {
        self.rtt_var
    }

    /// RTT, in microseconds — the ACK CIF's wire unit (§3.2.4, rule 31).
    pub fn rtt_us(&self) -> u32 {
        self.rtt.as_micros().min(u128::from(u32::MAX)) as u32
    }

    /// RTTVar, in microseconds — the ACK CIF's wire unit (§3.2.4, rule 31).
    pub fn rtt_var_us(&self) -> u32 {
        self.rtt_var.as_micros().min(u128::from(u32::MAX)) as u32
    }

    /// Feed one round-trip `sample` (rules 29-30's `rtt`), updating RTT and
    /// RTTVar from the *current* (pre-update) values.
    pub fn update(&mut self, sample: Duration) {
        let rtt_us = self.rtt.as_micros() as i64;
        let rtt_var_us = self.rtt_var.as_micros() as i64;
        let sample_us = sample.as_micros() as i64;

        // rule 29: RTT = 7/8 * RTT + 1/8 * rtt
        let new_rtt_us = (7 * rtt_us + sample_us) / 8;
        // rule 30: RTTVar = 3/4 * RTTVar + 1/4 * abs(RTT - rtt)
        let new_rtt_var_us = (3 * rtt_var_us + (rtt_us - sample_us).abs()) / 4;

        self.rtt = Duration::from_micros(new_rtt_us.max(0) as u64);
        self.rtt_var = Duration::from_micros(new_rtt_var_us.max(0) as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_values_match_rule_31() {
        let e = RttEstimator::new();
        assert_eq!(e.rtt(), Duration::from_millis(100));
        assert_eq!(e.rtt_var(), Duration::from_millis(50));
        assert_eq!(e.rtt_us(), 100_000);
        assert_eq!(e.rtt_var_us(), 50_000);
    }

    #[test]
    fn one_update_matches_hand_computed_formula() {
        let mut e = RttEstimator::new();
        // RTT=100_000us, RTTVar=50_000us, sample=20_000us.
        // new_RTT = (7*100_000 + 20_000) / 8 = 90_000
        // new_RTTVar = (3*50_000 + |100_000-20_000|) / 4 = (150_000+80_000)/4 = 57_500
        e.update(Duration::from_micros(20_000));
        assert_eq!(e.rtt_us(), 90_000);
        assert_eq!(e.rtt_var_us(), 57_500);
    }

    #[test]
    fn matching_sample_shrinks_rttvar_and_holds_rtt() {
        let mut e = RttEstimator::new();
        // sample == current RTT: RTT is unchanged, RTTVar decays toward 0.
        e.update(Duration::from_micros(100_000));
        assert_eq!(e.rtt_us(), 100_000);
        assert_eq!(e.rtt_var_us(), 37_500); // (3*50_000 + 0) / 4
    }

    #[test]
    fn repeated_sampling_converges_toward_the_injected_rtt() {
        let mut e = RttEstimator::new();
        let target = Duration::from_millis(30);
        for _ in 0..40 {
            e.update(target);
        }
        let got = e.rtt().as_micros() as i64;
        let want = target.as_micros() as i64;
        assert!(
            (got - want).abs() < 2_000,
            "expected convergence within 2ms of {want}us, got {got}us"
        );
    }
}
