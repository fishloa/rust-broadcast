//! The in-flight window of ordered resources and the lanes their requests queue in.
//!
//! [`Window`] counts a resource from the moment it is issued until it is delivered, holds bodies
//! that arrive early so they are delivered in issue order, and remembers which resources failed so
//! delivery stalls on them instead of skipping ahead. [`Lanes`] orders queued actions by the
//! sequence they were produced in while letting only fresh requests be held back by a full window,
//! so a retry inside the window is never blocked.

use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::hash::Hash;
use std::collections::HashMap;

#[derive(Debug)]
pub(crate) struct Window<K> {
    cap: usize,
    order: VecDeque<K>,
    held: HashMap<K, Vec<u8>>,
    failed: BTreeSet<K>,
}

impl<K: Copy + Eq + Ord + Hash> Window<K> {
    /// A window of `cap` resources; a cap of zero is raised to one.
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            order: VecDeque::new(),
            held: HashMap::new(),
            failed: BTreeSet::new(),
        }
    }

    /// Room for another fresh request: fewer than `cap` resources issued and not yet delivered.
    pub(crate) fn is_open(&self) -> bool {
        self.order.len() < self.cap
    }

    pub(crate) fn contains(&self, key: &K) -> bool {
        self.order.contains(key)
    }

    /// Puts `key` last in delivery order, unless it is already in the window, and clears its
    /// failed mark.
    pub(crate) fn issue(&mut self, key: K) {
        self.failed.remove(&key);
        if !self.order.contains(&key) {
            self.order.push_back(key);
        }
    }

    pub(crate) fn hold(&mut self, key: K, body: Vec<u8>) {
        self.held.insert(key, body);
    }

    /// The next resource in delivery order if its body is held and it has not failed.
    pub(crate) fn pop_ready(&mut self) -> Option<(K, Vec<u8>)> {
        let front = self.order.front().copied()?;
        if self.failed.contains(&front) {
            return None;
        }
        let body = self.held.remove(&front)?;
        self.order.pop_front();
        Some((front, body))
    }

    pub(crate) fn fail(&mut self, key: K) {
        self.failed.insert(key);
    }

    /// Puts a delivered-then-rejected resource back first in delivery order, marked failed.
    pub(crate) fn requeue(&mut self, key: K) {
        self.order.push_front(key);
        self.failed.insert(key);
    }

    /// Forgets `key` entirely: its place in order, its held body and its failed mark.
    pub(crate) fn retire(&mut self, key: &K) {
        self.failed.remove(key);
        self.order.retain(|o| o != key);
        self.held.remove(key);
    }

    pub(crate) fn failed(&self) -> Vec<K> {
        self.failed.iter().copied().collect()
    }

    pub(crate) fn clear(&mut self) {
        self.order.clear();
        self.held.clear();
        self.failed.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    Other,
    Retry,
    Fresh,
}

#[derive(Debug)]
pub(crate) struct Lanes<A> {
    seq: u64,
    other: VecDeque<(u64, A)>,
    retry: VecDeque<(u64, A)>,
    fresh: VecDeque<(u64, A)>,
}

impl<A> Lanes<A> {
    pub(crate) fn new() -> Self {
        Self {
            seq: 0,
            other: VecDeque::new(),
            retry: VecDeque::new(),
            fresh: VecDeque::new(),
        }
    }

    pub(crate) fn push(&mut self, lane: Lane, action: A) {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        let queue = match lane {
            Lane::Other => &mut self.other,
            Lane::Retry => &mut self.retry,
            Lane::Fresh => &mut self.fresh,
        };
        queue.push_back((seq, action));
    }

    /// The oldest queued action across the lanes; the fresh lane is skipped while the window is
    /// full.
    pub(crate) fn next(&mut self, window_open: bool) -> Option<A> {
        let head = |queue: &VecDeque<(u64, A)>| queue.front().map(|(seq, _)| *seq);
        let lane = [
            (head(&self.other), Lane::Other),
            (head(&self.retry), Lane::Retry),
            (head(&self.fresh).filter(|_| window_open), Lane::Fresh),
        ]
        .into_iter()
        .filter_map(|(seq, lane)| seq.map(|seq| (seq, lane)))
        .min_by_key(|(seq, _)| *seq)
        .map(|(_, lane)| lane)?;
        let queue = match lane {
            Lane::Other => &mut self.other,
            Lane::Retry => &mut self.retry,
            Lane::Fresh => &mut self.fresh,
        };
        queue.pop_front().map(|(_, action)| action)
    }

    pub(crate) fn any_other(&self, mut pred: impl FnMut(&A) -> bool) -> bool {
        self.other.iter().any(|(_, a)| pred(a))
    }

    /// The queued retry and fresh actions, retries first.
    pub(crate) fn resource_actions(&self) -> impl Iterator<Item = &A> {
        self.retry.iter().chain(self.fresh.iter()).map(|(_, a)| a)
    }

    pub(crate) fn clear(&mut self) {
        self.other.clear();
        self.retry.clear();
        self.fresh.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_open_while_fewer_than_cap_are_in_flight() {
        let mut window = Window::new(2);
        assert!(window.is_open());
        window.issue(1u32);
        assert!(window.is_open());
        window.issue(2);
        assert!(!window.is_open(), "cap reached");
        window.retire(&1);
        assert!(window.is_open());
    }

    #[test]
    fn a_zero_cap_is_one() {
        let mut window = Window::new(0);
        assert!(window.is_open());
        window.issue(1u32);
        assert!(!window.is_open());
    }

    #[test]
    fn a_resource_is_in_flight_until_delivered_even_after_its_body_arrived() {
        let mut window = Window::new(1);
        window.issue(1u32);
        window.hold(1, vec![1]);
        assert!(!window.is_open(), "held but not yet delivered still counts");
        assert!(window.pop_ready().is_some());
        assert!(window.is_open());
    }

    #[test]
    fn issuing_twice_does_not_take_a_second_slot() {
        let mut window = Window::new(2);
        window.issue(1u32);
        window.issue(1);
        assert!(window.is_open());
    }

    #[test]
    fn bodies_are_delivered_in_issue_order_whatever_the_arrival_order() {
        let mut window = Window::new(8);
        for key in [1u32, 2, 3] {
            window.issue(key);
        }
        window.hold(3, vec![3]);
        window.hold(2, vec![2]);
        assert!(window.pop_ready().is_none(), "1 has not arrived");
        window.hold(1, vec![1]);
        let delivered: Vec<(u32, Vec<u8>)> = std::iter::from_fn(|| window.pop_ready()).collect();
        assert_eq!(delivered, vec![(1, vec![1]), (2, vec![2]), (3, vec![3])]);
        assert!(window.pop_ready().is_none());
    }

    #[test]
    fn a_failed_front_blocks_delivery_until_it_is_reissued() {
        let mut window = Window::new(8);
        window.issue(1u32);
        window.issue(2);
        window.hold(1, vec![1]);
        window.hold(2, vec![2]);
        window.fail(1);
        assert!(
            window.pop_ready().is_none(),
            "1 failed, 2 must not overtake it"
        );
        window.issue(1);
        assert_eq!(window.pop_ready(), Some((1, vec![1])));
        assert_eq!(window.pop_ready(), Some((2, vec![2])));
    }

    #[test]
    fn reissuing_keeps_the_original_place_in_order() {
        let mut window = Window::new(8);
        window.issue(1u32);
        window.issue(2);
        window.fail(1);
        window.issue(1);
        window.hold(2, vec![2]);
        assert!(
            window.pop_ready().is_none(),
            "1 is still first and has no body"
        );
        window.hold(1, vec![1]);
        assert_eq!(window.pop_ready().map(|(k, _)| k), Some(1));
    }

    #[test]
    fn a_requeued_resource_goes_first_and_is_failed() {
        let mut window = Window::new(8);
        window.issue(2u32);
        window.requeue(1);
        window.hold(1, vec![1]);
        assert!(window.pop_ready().is_none(), "requeue marks it failed");
        assert!(window.contains(&1));
        window.issue(1);
        assert_eq!(window.pop_ready().map(|(k, _)| k), Some(1));
    }

    #[test]
    fn retire_forgets_order_body_and_failed_mark() {
        let mut window = Window::new(8);
        window.issue(1u32);
        window.issue(2);
        window.hold(1, vec![1]);
        window.fail(1);
        window.retire(&1);
        assert!(!window.contains(&1));
        assert!(window.failed().is_empty());
        window.hold(2, vec![2]);
        assert_eq!(window.pop_ready(), Some((2, vec![2])));
        window.issue(1);
        assert!(window.pop_ready().is_none(), "the retired body is gone");
    }

    #[test]
    fn failed_lists_every_failed_resource_once() {
        let mut window = Window::new(8);
        window.fail(3u32);
        window.fail(1);
        window.fail(3);
        assert_eq!(window.failed(), vec![1, 3]);
    }

    #[test]
    fn clear_empties_the_window() {
        let mut window = Window::new(1);
        window.issue(1u32);
        window.hold(1, vec![1]);
        window.fail(1);
        window.clear();
        assert!(window.is_open());
        assert!(window.failed().is_empty());
        assert!(window.pop_ready().is_none());
    }

    fn drain(lanes: &mut Lanes<u32>, window_open: bool) -> Vec<u32> {
        std::iter::from_fn(|| lanes.next(window_open)).collect()
    }

    #[test]
    fn lanes_yield_in_the_order_pushed_across_lanes() {
        let mut lanes = Lanes::new();
        lanes.push(Lane::Fresh, 1);
        lanes.push(Lane::Other, 2);
        lanes.push(Lane::Retry, 3);
        lanes.push(Lane::Fresh, 4);
        assert_eq!(drain(&mut lanes, true), vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_full_window_holds_fresh_but_never_other_or_retry() {
        let mut lanes = Lanes::new();
        lanes.push(Lane::Fresh, 1);
        lanes.push(Lane::Retry, 2);
        lanes.push(Lane::Other, 3);
        assert_eq!(drain(&mut lanes, false), vec![2, 3]);
        assert_eq!(
            drain(&mut lanes, true),
            vec![1],
            "released once the window opens"
        );
    }

    #[test]
    fn a_held_fresh_head_does_not_block_a_later_lane() {
        let mut lanes = Lanes::new();
        lanes.push(Lane::Fresh, 1);
        lanes.push(Lane::Fresh, 2);
        lanes.push(Lane::Retry, 3);
        assert_eq!(lanes.next(false), Some(3));
        assert_eq!(lanes.next(false), None);
        assert_eq!(drain(&mut lanes, true), vec![1, 2]);
    }

    #[test]
    fn sequence_numbers_wrap_without_panicking() {
        let mut lanes = Lanes::new();
        lanes.seq = u64::MAX;
        lanes.push(Lane::Other, 1);
        lanes.push(Lane::Other, 2);
        assert_eq!(lanes.seq, 1);
        assert_eq!(lanes.next(true), Some(1));
    }

    #[test]
    fn any_other_looks_only_at_the_other_lane() {
        let mut lanes = Lanes::new();
        lanes.push(Lane::Fresh, 7);
        lanes.push(Lane::Retry, 7);
        assert!(!lanes.any_other(|a| *a == 7));
        lanes.push(Lane::Other, 7);
        assert!(lanes.any_other(|a| *a == 7));
    }

    #[test]
    fn resource_actions_are_the_retry_and_fresh_lanes() {
        let mut lanes = Lanes::new();
        lanes.push(Lane::Other, 1);
        lanes.push(Lane::Fresh, 2);
        lanes.push(Lane::Retry, 3);
        assert_eq!(
            lanes.resource_actions().copied().collect::<Vec<_>>(),
            vec![3, 2]
        );
    }

    #[test]
    fn clear_empties_every_lane() {
        let mut lanes = Lanes::new();
        lanes.push(Lane::Other, 1);
        lanes.push(Lane::Retry, 2);
        lanes.push(Lane::Fresh, 3);
        lanes.clear();
        assert!(drain(&mut lanes, true).is_empty());
    }
}
