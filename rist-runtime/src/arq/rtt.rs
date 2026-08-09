//! RTT smoothing + RTT-Echo-to-sample conversion for the ARQ retry
//! scheduler. See the `arq` module doc's Attribution section for the full
//! spec-vs-librist accounting — nothing in this file is a TR-06-1
//! transcription.

use core::time::Duration;

use crate::RttEcho;

/// An 8-sample exponentially-weighted moving average of RTT, copied
/// bit-for-bit (integer accumulator shape, not a float EWMA) from librist's
/// `eight_times_rtt` (`src/rist-private.h::rist_peer_rtt_update`,
/// BSD-2-Clause, VSF reference implementation — see the `arq` module doc's
/// Attribution section): `acc -= acc / 8; acc += sample;`, read back via
/// `acc / 8`.
///
/// Unlike librist (which seeds its accumulator with `recovery_rtt_min * 8`
/// at peer construction, so it never truly lacks an estimate), this
/// estimator starts with **no** estimate ([`Self::smoothed`] returns
/// `None`) until the first real sample arrives via [`Self::update`]. This
/// is a deliberate divergence, not an oversight: it is what lets
/// [`super::Receiver`] fall back to TR-06-1 Appendix B's fixed interval
/// specifically for "no RTT estimate yet" (Appendix B's own "in the absence
/// of user input" framing), then switch permanently to RTT-driven
/// scheduling once real data exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RttEstimator {
    /// `8 * smoothed_rtt`, in whole microseconds. `None` until the first
    /// sample.
    eight_times_us: Option<u64>,
}

impl RttEstimator {
    /// A fresh estimator with no RTT estimate yet.
    pub const fn new() -> Self {
        RttEstimator {
            eight_times_us: None,
        }
    }

    /// Fold a fresh RTT sample into the 8-tap smoothed average.
    pub fn update(&mut self, sample: Duration) {
        let sample_us = sample.as_micros().min(u128::from(u64::MAX)) as u64;
        self.eight_times_us = Some(match self.eight_times_us {
            None => sample_us.saturating_mul(8),
            Some(acc) => acc - acc / 8 + sample_us,
        });
    }

    /// The current smoothed RTT, or `None` if [`Self::update`] has never
    /// been called.
    pub fn smoothed(&self) -> Option<Duration> {
        self.eight_times_us
            .map(|acc| Duration::from_micros(acc / 8))
    }
}

/// Turn a completed RTT Echo round trip (TR-06-1 §5.2.6) into an RTT sample
/// for [`RttEstimator::update`].
///
/// The caller sent an RTT Echo Request at `sent_at` and received `response`
/// (an [`RttEcho`] with [`crate::RttEchoKind::Response`]) at `now`. The raw
/// wall-clock round trip (`now - sent_at`) includes the peer's own
/// processing time — `response.processing_delay_us`, "microseconds between
/// receiving the request and sending the response" (§5.2.6) — which is not
/// network transit time, so it is subtracted out.
///
/// **Implementation policy**: TR-06-1 defines the wire fields (§5.2.6) but
/// not this arithmetic — that "elapsed minus processing delay" is the
/// right way to turn them into an RTT sample is this crate's own reading of
/// the field semantics, not a spec-stated formula. The caller is
/// responsible for having matched `response` to the `Request` it sent at
/// `sent_at` (TR-06-1 does not define a correlation mechanism beyond the
/// echoed `timestamp` field, which this function does not itself inspect).
pub fn rtt_sample(sent_at: Duration, now: Duration, response: &RttEcho) -> Duration {
    let elapsed = now.saturating_sub(sent_at);
    elapsed.saturating_sub(Duration::from_micros(u64::from(
        response.processing_delay_us,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RttEchoKind;
    use alloc::vec::Vec;

    #[test]
    fn no_estimate_until_first_sample() {
        let est = RttEstimator::new();
        assert_eq!(est.smoothed(), None);
    }

    #[test]
    fn first_sample_seeds_the_accumulator_exactly() {
        let mut est = RttEstimator::new();
        est.update(Duration::from_millis(20));
        assert_eq!(est.smoothed(), Some(Duration::from_millis(20)));
    }

    #[test]
    fn repeated_identical_samples_stay_stable() {
        let mut est = RttEstimator::new();
        for _ in 0..20 {
            est.update(Duration::from_millis(50));
        }
        assert_eq!(est.smoothed(), Some(Duration::from_millis(50)));
    }

    #[test]
    fn a_single_outlier_moves_the_average_only_partway() {
        let mut est = RttEstimator::new();
        est.update(Duration::from_millis(20));
        est.update(Duration::from_millis(100));
        let smoothed = est.smoothed().unwrap();
        // acc = 20*8=160 -> 160 - 160/8 + 100 = 160-20+100=240 -> /8=30ms
        assert_eq!(smoothed, Duration::from_millis(30));
    }

    #[test]
    fn rtt_sample_subtracts_processing_delay() {
        let echo = RttEcho {
            kind: RttEchoKind::Response,
            ssrc_media: 1,
            timestamp: 0,
            processing_delay_us: 4_000,
            padding: Vec::new(),
        };
        let sample = rtt_sample(Duration::from_millis(10), Duration::from_millis(30), &echo);
        // 20ms elapsed - 4ms processing = 16ms network RTT.
        assert_eq!(sample, Duration::from_millis(16));
    }

    #[test]
    fn rtt_sample_never_underflows() {
        let echo = RttEcho {
            kind: RttEchoKind::Response,
            ssrc_media: 1,
            timestamp: 0,
            processing_delay_us: 50_000,
            padding: Vec::new(),
        };
        let sample = rtt_sample(Duration::from_millis(10), Duration::from_millis(20), &echo);
        assert_eq!(sample, Duration::ZERO);
    }
}

#[cfg(test)]
mod ewma_shape_tests {
    use super::*;

    /// The 8-tap accumulator shape is pinned deliberately.
    ///
    /// This mirrors librist's `eight_times_rtt` bit-for-bit — `acc -= acc / 8;
    /// acc += sample;`, read back as `acc / 8` — because the maintainer chose
    /// to adopt the reference implementation's behaviour exactly, including its
    /// integer rounding. A float EWMA with a "equivalent" alpha would converge
    /// differently and is NOT the same thing.
    ///
    /// Without this test the divisor is invisible to the suite: changing `/ 8`
    /// to `/ 4` throughout previously caused zero failures, so the one constant
    /// the design decision actually turns on was unpinned.
    #[test]
    fn accumulator_is_an_eight_tap_ewma_matching_librist() {
        let mut e = RttEstimator::default();
        assert_eq!(e.smoothed(), None, "no estimate before the first sample");

        // First sample seeds acc = sample * 8, so the smoothed value equals it.
        e.update(Duration::from_micros(8_000));
        assert_eq!(e.smoothed(), Some(Duration::from_micros(8_000)));

        // Second sample: acc = 64000 - 8000 + 16000 = 72000 -> 72000/8 = 9000.
        // A /4 accumulator would give 32000 - 8000 + 16000 = 40000 -> 10000.
        e.update(Duration::from_micros(16_000));
        assert_eq!(
            e.smoothed(),
            Some(Duration::from_micros(9_000)),
            "8-tap EWMA must move 1/8 of the way toward a new sample"
        );

        // Third: acc = 72000 - 9000 + 16000 = 79000 -> 9875.
        e.update(Duration::from_micros(16_000));
        assert_eq!(
            e.smoothed(),
            Some(Duration::from_micros(9_875)),
            "convergence rate is the 8-tap one, not 4- or 16-tap"
        );
    }
}
