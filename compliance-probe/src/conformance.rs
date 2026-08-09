//! PCR interval-error ("drift") and jitter tracking, per PID.
//!
//! ETSI TR 101 290 §5.2 Table 5.0b indicator 2.4 (`PCR_accuracy_error`) —
//! whether the *value* a PCR carries is within ±500 ns of the actual 27 MHz
//! System Clock (ISO/IEC 13818-1 §2.4.2.2) — needs a hardware reference clock
//! this sans-IO probe does not have; `dvb-conformance` documents this and
//! never emits that indicator (see its README, "Not implemented"). What
//! *is* honestly computable from a caller-supplied
//! [`core::time::Duration`] arrival clock is a coarser, different question:
//! "does the *rate* at which this PID's PCR advances match the rate at which
//! packets are actually arriving" — a real signal for encoder/multiplexer
//! clock-rate mismatches and network-induced delivery-rate variation, just
//! not a spec-conformant 2.4. This module computes exactly that, and names
//! its metrics accordingly (`compliance_probe_pcr_drift_ppm`, not
//! `pcr_accuracy_error`) so a dashboard never implies the stricter check ran.

use alloc::collections::BTreeMap;
use core::time::Duration;

/// PCR clock rate — ISO/IEC 13818-1 §2.4.2.2, "27 MHz System Clock".
const PCR_HZ: u64 = 27_000_000;

/// PCR field modulus: the 33-bit `program_clock_reference_base` plus the
/// 9-bit `program_clock_reference_extension` (ISO/IEC 13818-1 §2.4.2.2)
/// together count `base * 300 + extension` 27 MHz ticks, wrapping at
/// `2^33 * 300`.
const PCR_MODULUS: u64 = (1u64 << 33) * 300;

/// A discontinuity guard, not a spec value: an inter-PCR gap this large
/// (wall-clock or PCR-derived) is treated as a legitimate discontinuity
/// (stream cut, encoder restart, or a caller-clock jump) rather than measured
/// as drift — the alternative is reporting an implausible multi-million-ppm
/// spike across a gap that was never a rate error to begin with.
const DISCONTINUITY_GUARD: Duration = Duration::from_secs(10);

/// Exponential-moving-average smoothing factor for the drift signal — an
/// engineering choice (heavier weight on history, so a single noisy interval
/// does not swing the reported drift), not a spec constant.
const DRIFT_EWMA_ALPHA: f64 = 0.1;

/// One PID's running PCR/arrival state.
struct PidState {
    last_pcr_27mhz: u64,
    last_arrival: Duration,
    drift_ewma_ppm: f64,
    have_drift: bool,
}

/// The result of observing one new PCR value on a PID, once at least a
/// second PCR has been seen (the first is a baseline with nothing to compare
/// against).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PcrSample {
    /// The PID this sample concerns.
    pub pid: u16,
    /// Smoothed (EWMA) interval-error, parts-per-million: positive means the
    /// PCR is advancing faster than wall-clock arrival time, negative slower.
    pub drift_ppm: f64,
    /// `|latest interval-error - drift_ppm|` — how far this specific interval
    /// deviated from the smoothed baseline.
    pub jitter_ppm: f64,
}

/// Per-PID PCR interval-error tracker. See the module docs for what this
/// does and does not measure.
#[derive(Default)]
pub(crate) struct PcrTracker {
    pids: BTreeMap<u16, PidState>,
}

impl PcrTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Observe one PCR value (`pcr_27mhz`, already `Pcr::as_27mhz()`) on
    /// `pid`, arriving at wall-clock `t`. Returns `None` on the first PCR
    /// seen for this PID (nothing to compare against yet), or when the
    /// interval since the last PCR looks like a discontinuity rather than a
    /// measurable rate — see [`DISCONTINUITY_GUARD`].
    pub(crate) fn observe(&mut self, pid: u16, pcr_27mhz: u64, t: Duration) -> Option<PcrSample> {
        let state = self.pids.entry(pid).or_insert(PidState {
            last_pcr_27mhz: pcr_27mhz,
            last_arrival: t,
            drift_ewma_ppm: 0.0,
            have_drift: false,
        });

        // First observation for this PID: record the baseline, nothing to
        // compare yet.
        if !state.have_drift && state.last_pcr_27mhz == pcr_27mhz && state.last_arrival == t {
            return None;
        }

        let pcr_delta_ticks = pcr_27mhz.wrapping_sub(state.last_pcr_27mhz) % PCR_MODULUS;
        let pcr_delta = Duration::from_secs_f64(pcr_delta_ticks as f64 / PCR_HZ as f64);
        let wall_delta = t.saturating_sub(state.last_arrival);

        state.last_pcr_27mhz = pcr_27mhz;
        state.last_arrival = t;

        if wall_delta.is_zero()
            || wall_delta > DISCONTINUITY_GUARD
            || pcr_delta > DISCONTINUITY_GUARD
        {
            // Non-monotonic/zero caller clock, or a gap wide enough to be a
            // real discontinuity rather than a rate to measure — do not
            // fabricate a drift number across it.
            state.have_drift = false;
            return None;
        }

        let error_ppm = (pcr_delta.as_secs_f64() - wall_delta.as_secs_f64())
            / wall_delta.as_secs_f64()
            * 1_000_000.0;

        let jitter_ppm = if state.have_drift {
            (error_ppm - state.drift_ewma_ppm).abs()
        } else {
            0.0
        };
        state.drift_ewma_ppm = if state.have_drift {
            DRIFT_EWMA_ALPHA * error_ppm + (1.0 - DRIFT_EWMA_ALPHA) * state.drift_ewma_ppm
        } else {
            error_ppm
        };
        state.have_drift = true;

        Some(PcrSample {
            pid,
            drift_ppm: state.drift_ewma_ppm,
            jitter_ppm,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A perfectly steady 27 MHz PCR advancing in lock-step with wall-clock
    /// arrival must settle to ~0 ppm drift and ~0 jitter.
    #[test]
    fn steady_pcr_settles_near_zero_drift() {
        let mut tracker = PcrTracker::new();
        let pid = 0x0100;
        let mut pcr = 0u64;
        let mut t = Duration::ZERO;
        let mut last = None;
        for _ in 0..20 {
            pcr += PCR_HZ / 10; // exactly 100ms worth of 27MHz ticks
            t += Duration::from_millis(100);
            last = tracker.observe(pid, pcr, t);
        }
        let sample = last.expect("must have a sample after >1 observation");
        assert!(
            sample.drift_ppm.abs() < 1.0,
            "expected ~0 ppm drift, got {}",
            sample.drift_ppm
        );
        assert!(
            sample.jitter_ppm.abs() < 1.0,
            "expected ~0 ppm jitter, got {}",
            sample.jitter_ppm
        );
    }

    /// A PCR running measurably fast relative to wall-clock arrival (encoder
    /// clock running hot) must show up as positive drift of the right
    /// magnitude — this is the bite: mutating the tracker to compare the
    /// wrong two durations, or to drop the `* 1_000_000.0` scale, changes
    /// this number.
    #[test]
    fn fast_pcr_reports_positive_drift_of_expected_magnitude() {
        let mut tracker = PcrTracker::new();
        let pid = 0x0100;
        // 27,002,700 Hz signalled against a 27,000,000 Hz wall-clock
        // reference: exactly +100 ppm high.
        let signalled_hz = PCR_HZ + 2_700;
        let mut pcr = 0u64;
        let mut t = Duration::ZERO;
        let mut last = None;
        for _ in 0..30 {
            pcr += signalled_hz / 10;
            t += Duration::from_millis(100);
            last = tracker.observe(pid, pcr, t);
        }
        let sample = last.unwrap();
        assert!(
            (sample.drift_ppm - 100.0).abs() < 1.0,
            "expected ~100 ppm drift, got {}",
            sample.drift_ppm
        );
    }

    /// A first observation on a PID has nothing to compare against and must
    /// not fabricate a sample.
    #[test]
    fn first_observation_yields_no_sample() {
        let mut tracker = PcrTracker::new();
        assert_eq!(
            tracker.observe(0x0100, 12_345, Duration::from_millis(1)),
            None
        );
    }

    /// A multi-second gap (stream cut / restart) must be treated as a
    /// discontinuity, not measured as an implausible drift spike.
    #[test]
    fn large_gap_is_treated_as_discontinuity_not_drift() {
        let mut tracker = PcrTracker::new();
        let pid = 0x0100;
        assert_eq!(tracker.observe(pid, 0, Duration::ZERO), None);
        // 20 real seconds elapse with only a small PCR advance recorded —
        // this must not be reported as a multi-million-ppm drift.
        let sample = tracker.observe(pid, PCR_HZ / 10, Duration::from_secs(20));
        assert!(
            sample.is_none(),
            "expected a suppressed discontinuity, got {sample:?}"
        );
    }

    /// PIDs are tracked independently: a fast PID and a steady PID must not
    /// influence each other's drift.
    #[test]
    fn pids_are_tracked_independently() {
        let mut tracker = PcrTracker::new();
        let steady_pid = 0x0100;
        let fast_pid = 0x0200;
        let mut steady_pcr = 0u64;
        let mut fast_pcr = 0u64;
        let mut t = Duration::ZERO;
        let mut steady_sample = None;
        let mut fast_sample = None;
        for _ in 0..20 {
            steady_pcr += PCR_HZ / 10;
            fast_pcr += (PCR_HZ + 27_000) / 10; // +1000 ppm
            t += Duration::from_millis(100);
            steady_sample = tracker.observe(steady_pid, steady_pcr, t);
            fast_sample = tracker.observe(fast_pid, fast_pcr, t);
        }
        assert!(steady_sample.unwrap().drift_ppm.abs() < 1.0);
        assert!((fast_sample.unwrap().drift_ppm - 1000.0).abs() < 5.0);
    }
}
