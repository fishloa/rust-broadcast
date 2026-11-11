//! Failure bookkeeping: per-resource consecutive failures, single-purpose attempt counters, and
//! the give-up verdict.

use core::hash::Hash;
use std::collections::HashMap;

/// The consecutive failures recorded for one resource and the status of the latest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) count: u32,
    pub(crate) status: Option<u16>,
}

/// Whether a resource that has failed `count` times in a row is tried again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Retry,
    GiveUp,
}

/// Consecutive failures per resource. A count is cleared only by [`FailureCounter::clear`] (the
/// resource was delivered, left the window, or was given up on), never by an unrelated delivery
/// such as a playlist or manifest refresh.
#[derive(Debug)]
pub(crate) struct FailureCounter<K> {
    failures: HashMap<K, Failure>,
}

impl<K: Copy + Eq + Hash> FailureCounter<K> {
    pub(crate) fn new() -> Self {
        Self {
            failures: HashMap::new(),
        }
    }

    pub(crate) fn record(&mut self, key: K, status: Option<u16>) -> Failure {
        let entry = self.failures.entry(key).or_insert(Failure {
            count: 0,
            status: None,
        });
        entry.count = entry.count.saturating_add(1);
        entry.status = status;
        *entry
    }

    pub(crate) fn clear(&mut self, key: &K) {
        self.failures.remove(key);
    }

    pub(crate) fn clear_all(&mut self) {
        self.failures.clear();
    }
}

/// `count` is how many failures in a row have been recorded. `max == 0` makes the first failure final; `u32::MAX` never gives up in practice.
pub(crate) fn verdict(count: u64, max: u32) -> Verdict {
    if count >= u64::from(max) {
        Verdict::GiveUp
    } else {
        Verdict::Retry
    }
}

/// A saturating attempt counter for one retry loop.
#[derive(Debug, Default)]
pub(crate) struct Attempts(u64);

impl Attempts {
    pub(crate) fn bump(&mut self) -> u64 {
        self.0 = self.0.saturating_add(1);
        self.0
    }

    pub(crate) fn reset(&mut self) {
        self.0 = 0;
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resource_counts_its_own_consecutive_failures() {
        let mut counter = FailureCounter::new();
        assert_eq!(counter.record(1u32, Some(503)).count, 1);
        assert_eq!(counter.record(1, Some(503)).count, 2);
        assert_eq!(
            counter.record(2, None).count,
            1,
            "another resource starts at one"
        );
        assert_eq!(counter.record(1, Some(503)).count, 3);
    }

    #[test]
    fn the_latest_status_wins_including_none() {
        let mut counter = FailureCounter::new();
        counter.record(1u32, Some(404));
        let latest = counter.record(1, None);
        assert_eq!(
            latest,
            Failure {
                count: 2,
                status: None
            }
        );
        assert_eq!(counter.record(1, Some(500)).status, Some(500));
    }

    #[test]
    fn clear_restarts_one_resource_and_leaves_the_others() {
        let mut counter = FailureCounter::new();
        counter.record(1u32, None);
        counter.record(1, None);
        counter.record(2, None);
        counter.clear(&1);
        assert_eq!(counter.record(1, None).count, 1);
        assert_eq!(counter.record(2, None).count, 2);
    }

    #[test]
    fn clearing_an_unknown_resource_is_a_no_op() {
        let mut counter = FailureCounter::new();
        counter.clear(&9u32);
        assert_eq!(counter.record(9, None).count, 1);
    }

    #[test]
    fn clear_all_restarts_every_resource() {
        let mut counter = FailureCounter::new();
        counter.record(1u32, None);
        counter.record(2, None);
        counter.clear_all();
        assert_eq!(counter.record(1, None).count, 1);
        assert_eq!(counter.record(2, None).count, 1);
    }

    #[test]
    fn a_count_saturates_instead_of_wrapping() {
        let mut counter = FailureCounter::new();
        counter.failures.insert(
            1u32,
            Failure {
                count: u32::MAX,
                status: None,
            },
        );
        assert_eq!(counter.record(1, None).count, u32::MAX);
    }

    #[test]
    fn the_verdict_gives_up_exactly_at_the_limit() {
        assert_eq!(verdict(1, 3), Verdict::Retry);
        assert_eq!(verdict(2, 3), Verdict::Retry);
        assert_eq!(verdict(3, 3), Verdict::GiveUp);
        assert_eq!(verdict(4, 3), Verdict::GiveUp);
    }

    #[test]
    fn a_zero_limit_makes_the_first_failure_final() {
        assert_eq!(verdict(1, 0), Verdict::GiveUp);
    }

    #[test]
    fn a_maximal_limit_keeps_retrying_for_any_realistic_count() {
        assert_eq!(verdict(1_000_000, u32::MAX), Verdict::Retry);
    }

    #[test]
    fn attempts_count_up_from_one_and_reset_to_zero() {
        let mut attempts = Attempts::default();
        assert!(attempts.is_zero());
        assert_eq!(attempts.bump(), 1);
        assert_eq!(attempts.bump(), 2);
        assert!(!attempts.is_zero());
        attempts.reset();
        assert!(attempts.is_zero());
        assert_eq!(attempts.bump(), 1);
    }

    #[test]
    fn attempts_saturate() {
        let mut attempts = Attempts(u64::MAX);
        assert_eq!(attempts.bump(), u64::MAX);
    }
}
