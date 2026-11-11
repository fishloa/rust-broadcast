//! Request deadlines with lazy deletion: a completed or replaced request leaves its entry in the
//! heap, and an entry only expires a request that is still live with exactly that deadline, so a
//! stale entry can never time out a newer request that reuses the key.

use alloc::collections::BinaryHeap;
use alloc::vec::Vec;
use core::cmp::Reverse;

pub(crate) const SLACK_FACTOR: usize = 4;
pub(crate) const SLACK_FLOOR: usize = 64;

#[derive(Debug)]
pub(crate) struct Deadlines<K> {
    heap: BinaryHeap<Reverse<(u64, K)>>,
}

impl<K: Copy + Ord> Deadlines<K> {
    pub(crate) fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
        }
    }

    pub(crate) fn arm(&mut self, key: K, deadline: u64) {
        self.heap.push(Reverse((deadline, key)));
    }

    /// Every key whose deadline is at or before `now` and for which `is_live(key, deadline)` holds,
    /// earliest first; entries examined are discarded either way.
    pub(crate) fn expire(&mut self, now: u64, mut is_live: impl FnMut(&K, u64) -> bool) -> Vec<K> {
        let mut expired = Vec::new();
        while let Some(Reverse((deadline, key))) = self.heap.peek().copied() {
            if deadline > now {
                break;
            }
            self.heap.pop();
            if is_live(&key, deadline) {
                expired.push(key);
            }
        }
        expired
    }

    /// Rebuilds the heap from `live` (the deadline of every live request) once stale entries
    /// outnumber what `live_len` live requests can account for, keeping it proportional.
    pub(crate) fn compact<I>(&mut self, live_len: usize, live: impl FnOnce() -> I)
    where
        I: IntoIterator<Item = (u64, K)>,
    {
        let limit = live_len
            .saturating_mul(SLACK_FACTOR)
            .saturating_add(SLACK_FLOOR);
        if self.heap.len() <= limit {
            return;
        }
        self.heap = live().into_iter().map(Reverse).collect();
    }

    pub(crate) fn clear(&mut self) {
        self.heap.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.heap.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always(_: &u32, _: u64) -> bool {
        true
    }

    #[test]
    fn nothing_expires_before_its_deadline_and_everything_does_at_it() {
        let mut deadlines = Deadlines::new();
        deadlines.arm(1u32, 100);
        assert!(deadlines.expire(99, always).is_empty());
        assert_eq!(
            deadlines.expire(100, always),
            vec![1],
            "expired at exactly the deadline"
        );
        assert!(
            deadlines.expire(1_000, always).is_empty(),
            "an entry expires once"
        );
    }

    #[test]
    fn expiry_is_earliest_first_and_leaves_later_deadlines() {
        let mut deadlines = Deadlines::new();
        deadlines.arm(3u32, 300);
        deadlines.arm(1, 100);
        deadlines.arm(2, 200);
        assert_eq!(deadlines.expire(250, always), vec![1, 2]);
        assert_eq!(deadlines.expire(300, always), vec![3]);
    }

    #[test]
    fn a_stale_entry_never_expires_a_reused_key() {
        let mut deadlines = Deadlines::new();
        deadlines.arm(7u32, 100);
        let live_deadline = 500;
        let expired = deadlines.expire(100, |_, deadline| deadline == live_deadline);
        assert!(
            expired.is_empty(),
            "the old entry is for a request that is gone"
        );
        deadlines.arm(7, live_deadline);
        assert_eq!(deadlines.expire(500, |_, d| d == live_deadline), vec![7]);
    }

    #[test]
    fn is_live_is_asked_with_the_entrys_own_key_and_deadline() {
        let mut deadlines = Deadlines::new();
        deadlines.arm(4u32, 10);
        deadlines.arm(5, 20);
        let mut seen = Vec::new();
        deadlines.expire(20, |key, deadline| {
            seen.push((*key, deadline));
            false
        });
        assert_eq!(seen, vec![(4, 10), (5, 20)]);
    }

    #[test]
    fn equal_deadlines_expire_in_key_order() {
        let mut deadlines = Deadlines::new();
        deadlines.arm(9u32, 10);
        deadlines.arm(2, 10);
        assert_eq!(deadlines.expire(10, always), vec![2, 9]);
    }

    #[test]
    fn compaction_keeps_the_heap_proportional_to_live_requests() {
        let mut deadlines = Deadlines::new();
        for key in 0..10_000u32 {
            deadlines.arm(key, u64::from(key));
            deadlines.compact(1, || [(u64::MAX, 0u32)]);
        }
        assert!(
            deadlines.len() <= SLACK_FLOOR + SLACK_FACTOR,
            "{} stale entries kept",
            deadlines.len()
        );
    }

    #[test]
    fn compaction_waits_until_the_slack_is_exceeded() {
        let mut deadlines = Deadlines::new();
        for key in 0..u32::try_from(SLACK_FLOOR).unwrap() {
            deadlines.arm(key, 1);
        }
        deadlines.compact(0, || -> Vec<(u64, u32)> {
            panic!("rebuilt at exactly the limit")
        });
        assert_eq!(deadlines.len(), SLACK_FLOOR);
        deadlines.arm(999, 1);
        deadlines.compact(0, || vec![(5u64, 1u32)]);
        assert_eq!(deadlines.len(), 1);
    }

    #[test]
    fn the_limit_grows_with_the_live_count() {
        let mut deadlines = Deadlines::new();
        for key in 0..u32::try_from(SLACK_FLOOR + SLACK_FACTOR).unwrap() {
            deadlines.arm(key, 1);
        }
        deadlines.compact(1, || -> Vec<(u64, u32)> {
            panic!("one live request allows the slack")
        });
        deadlines.arm(u32::MAX, 1);
        deadlines.compact(1, || vec![(2u64, 0u32)]);
        assert_eq!(deadlines.len(), 1);
    }

    #[test]
    fn a_rebuilt_heap_still_expires_the_live_entries() {
        let mut deadlines = Deadlines::new();
        for key in 0..200u32 {
            deadlines.arm(key, 1);
        }
        deadlines.compact(1, || [(50u64, 42u32)]);
        assert_eq!(deadlines.expire(50, always), vec![42]);
    }

    #[test]
    fn clear_forgets_everything() {
        let mut deadlines = Deadlines::new();
        deadlines.arm(1u32, 1);
        deadlines.clear();
        assert!(deadlines.expire(u64::MAX, always).is_empty());
    }
}
