//! NetEq-style forgetting histogram of per-interval maxima, Q15 weights.

use core::time::Duration;

use crate::config::{PERMILLE, Q15_SHIFT, Q15_UNIT};
use crate::time::window_micros;
use alloc::vec;
use alloc::vec::Vec;

/// Histogram of interval maxima with exponential forgetting.
///
/// Per [`add`](Self::add): every weight is multiplied by `f` (Q15, truncating
/// `>> 15`), then the value's bucket gains `1 - f`. `f` ramps as
/// `min(forget, k/(k+1))` over the first intervals so the first few behave as a
/// plain average (NetEq's start-weight idea).
#[derive(Clone, Debug)]
pub struct ForgettingHistogram {
    w: Vec<u32>,
    bucket_us: u64,
    forget_q15: u64,
    quantile_permille: u64,
    intervals: u64,
}

impl ForgettingHistogram {
    /// New histogram. `buckets` and `bucket` must be non-zero (validated
    /// by the configs; a zero width is clamped to one microsecond here so this
    /// never divides by zero).
    #[must_use]
    pub fn new(buckets: u16, bucket: Duration, forget_q15: u16, quantile_permille: u16) -> Self {
        Self {
            w: vec![0; usize::from(buckets).max(1)],
            bucket_us: window_micros(bucket).max(1),
            forget_q15: u64::from(forget_q15).min(Q15_UNIT - 1),
            quantile_permille: u64::from(quantile_permille),
            intervals: 0,
        }
    }

    /// Fold in one interval maximum (microseconds).
    pub fn add(&mut self, value_us: u64) {
        let k = self.intervals.min(u64::from(u32::MAX));
        let ramp = (k << Q15_SHIFT) / (k + 1);
        let f = self.forget_q15.min(ramp);
        for x in &mut self.w {
            *x = u32::try_from((u64::from(*x) * f) >> Q15_SHIFT).unwrap_or(u32::MAX);
        }
        let last = self.w.len() - 1;
        let idx = usize::try_from(value_us / self.bucket_us)
            .unwrap_or(usize::MAX)
            .min(last);
        let gain = u32::try_from(Q15_UNIT - f).unwrap_or(u32::MAX);
        self.w[idx] = self.w[idx].saturating_add(gain);
        self.intervals = self.intervals.saturating_add(1);
    }

    /// Smallest bucket upper edge whose cumulative weight reaches the
    /// quantile (`cum * 1000 >= total * q`); 0 for an empty histogram.
    #[must_use]
    pub fn quantile_us(&self) -> u64 {
        let total: u64 = self.w.iter().map(|&x| u64::from(x)).sum();
        if total == 0 {
            return 0;
        }
        let mut cum = 0u64;
        for (i, &x) in self.w.iter().enumerate() {
            cum += u64::from(x);
            if cum * PERMILLE >= total * self.quantile_permille {
                return (i as u64 + 1) * self.bucket_us;
            }
        }
        self.w.len() as u64 * self.bucket_us
    }

    /// Weight (Q15) of bucket `i`, 0 out of range.
    #[must_use]
    pub fn weight(&self, i: usize) -> u32 {
        self.w.get(i).copied().unwrap_or(0)
    }

    /// Sum of all weights.
    #[must_use]
    pub fn total_weight(&self) -> u64 {
        self.w.iter().map(|&x| u64::from(x)).sum()
    }

    /// Intervals folded in so far.
    #[must_use]
    pub fn intervals(&self) -> u64 {
        self.intervals
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.w.fill(0);
        self.intervals = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::ForgettingHistogram;
    use core::time::Duration;

    fn bucket() -> Duration {
        Duration::from_millis(10)
    }

    #[test]
    fn an_interval_weight_halves_after_one_more_interval() {
        let mut h = ForgettingHistogram::new(400, bucket(), 16_384, 1_000);
        h.add(10_000);
        assert_eq!(h.weight(1), 32_768);
        assert_eq!(h.total_weight(), 32_768);
        assert_eq!(h.intervals(), 1);
        h.add(20_000);
        assert_eq!(h.weight(1), 16_384);
        assert_eq!(h.weight(2), 16_384);
        h.add(10_000);
        assert_eq!(h.weight(1), 24_576);
        assert_eq!(h.weight(2), 8_192);
        assert_eq!(h.total_weight(), 32_768);
        assert_eq!(h.intervals(), 3);
    }

    #[test]
    fn the_quantile_lands_in_the_dominant_bucket() {
        let mut h = ForgettingHistogram::new(400, bucket(), 32_113, 990);
        for _ in 0..100 {
            h.add(10_000);
        }
        h.add(90_000);
        assert_eq!(h.intervals(), 101);
        assert_eq!(h.quantile_us(), 100_000);
        let mut h = ForgettingHistogram::new(400, bucket(), 32_113, 900);
        for _ in 0..100 {
            h.add(10_000);
        }
        h.add(90_000);
        assert_eq!(h.quantile_us(), 20_000);
    }

    #[test]
    fn a_quantile_above_one_thousand_falls_through_to_the_histogram_range() {
        let mut h = ForgettingHistogram::new(4, bucket(), 0, 1_001);
        h.add(10_000);
        assert_eq!(h.quantile_us(), 40_000);
    }
}
