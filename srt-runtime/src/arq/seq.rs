//! 32-bit wrap-safe sequence-number arithmetic for SRT ARQ.
//!
//! SRT data packet sequence numbers occupy the low 31 bits of the header's
//! first word (`draft-sharabayko-srt-01` §3.1, Figure 3: `Packet Sequence
//! Number (31)`; the top bit is the header `F` packet-type flag, reused
//! bit-for-bit as the NAK loss-list range marker, Appendix A) — see the
//! crate-internal `crate::packet::SEQ_NUMBER_MASK`.
//!
//! `specs/rules/srt-arq.md` does not specify a sequence-number comparison or
//! arithmetic algorithm for ARQ purposes (it is not one of the curated
//! rules). This module resolves that gap the standard way for a modular
//! sequence space — comparable to RFC 1982 serial number arithmetic:
//! circular over the 31-bit space, picking the shorter of the two
//! directions between two numbers as "before"/"after". This is
//! implementation-defined, not spec-cited, and is needed for the send/
//! receive buffers to behave correctly once a stream's sequence numbers
//! wrap past `0x7FFF_FFFF` back to `0`.

use crate::packet::SEQ_NUMBER_MASK;
use broadcast_common::seq::SeqSpace;

/// The SRT sequence-number space: 31 bits (§3.1). The arithmetic itself is
/// `broadcast_common::seq` (shared with `rist-runtime`, #1141), sized from
/// [`SEQ_NUMBER_MASK`].
const SPACE: SeqSpace = SeqSpace::new(SEQ_NUMBER_MASK.count_ones());

/// Add `n` to a sequence number, wrapping at the 31-bit boundary.
pub fn seq_add(seq: u32, n: u32) -> u32 {
    SPACE.add(seq, n)
}

/// The next sequence number after `seq` (wraps `0x7FFF_FFFF` -> `0`).
pub fn seq_next(seq: u32) -> u32 {
    seq_add(seq, 1)
}

/// Signed circular distance `a - b` in the 31-bit sequence space, in
/// `(-2^30, 2^30]`. Positive means `a` is ahead of `b`.
pub fn seq_diff(a: u32, b: u32) -> i32 {
    SPACE.diff(a, b)
}

/// `a` precedes `b` in circular sequence order.
pub fn seq_lt(a: u32, b: u32) -> bool {
    SPACE.lt(a, b)
}

/// `a` precedes or equals `b`.
pub fn seq_leq(a: u32, b: u32) -> bool {
    SPACE.leq(a, b)
}

/// `a` follows `b` in circular sequence order.
pub fn seq_gt(a: u32, b: u32) -> bool {
    SPACE.gt(a, b)
}

/// `a` follows or equals `b`.
pub fn seq_geq(a: u32, b: u32) -> bool {
    SPACE.geq(a, b)
}

/// `seq` lies within the inclusive circular range `first..=last`, walking
/// forward from `first`. A `last` that precedes `first` in circular order is
/// an empty (malformed) range and never matches.
pub fn seq_in_closed_range(seq: u32, first: u32, last: u32) -> bool {
    SPACE.in_closed_range(seq, first, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_mask_is_the_wire_mask() {
        assert_eq!(SPACE.mask(), SEQ_NUMBER_MASK);
    }

    #[test]
    fn wrap_boundary_increment() {
        assert_eq!(seq_next(SEQ_NUMBER_MASK), 0);
        assert_eq!(seq_add(SEQ_NUMBER_MASK, 1), 0);
        assert_eq!(seq_add(SEQ_NUMBER_MASK, 5), 4);
        assert_eq!(seq_add(0, SEQ_NUMBER_MASK), SEQ_NUMBER_MASK);
    }

    #[test]
    fn wrap_boundary_ordering() {
        // 0 comes right after SEQ_NUMBER_MASK (the maximum 31-bit value) in
        // circular sequence order.
        assert!(seq_lt(SEQ_NUMBER_MASK, 0));
        assert!(seq_gt(0, SEQ_NUMBER_MASK));
        assert_eq!(seq_diff(0, SEQ_NUMBER_MASK), 1);
        assert_eq!(seq_diff(SEQ_NUMBER_MASK, 0), -1);
    }

    #[test]
    fn ordinary_ordering_without_wrap() {
        assert!(seq_lt(10, 20));
        assert!(seq_leq(10, 10));
        assert!(seq_leq(10, 20));
        assert!(seq_gt(20, 10));
        assert!(seq_geq(20, 20));
        assert!(seq_geq(20, 10));
        assert!(!seq_lt(20, 10));
        assert!(!seq_gt(10, 20));
    }

    #[test]
    fn diff_is_antisymmetric() {
        for (a, b) in [(0u32, 0u32), (5, 100), (SEQ_NUMBER_MASK, 3), (12345, 12340)] {
            assert_eq!(seq_diff(a, b), -seq_diff(b, a));
        }
    }

    #[test]
    fn closed_range_membership_walks_forward_from_first() {
        assert!(seq_in_closed_range(5, 5, 10));
        assert!(seq_in_closed_range(10, 5, 10));
        assert!(seq_in_closed_range(7, 5, 10));
        assert!(!seq_in_closed_range(4, 5, 10));
        assert!(!seq_in_closed_range(11, 5, 10));
        // A `last` before `first` is a malformed (empty) range.
        assert!(!seq_in_closed_range(7, 10, 5));
        // Wrap-safe: the range straddles the 31-bit boundary.
        assert!(seq_in_closed_range(SEQ_NUMBER_MASK, SEQ_NUMBER_MASK - 2, 1));
        assert!(seq_in_closed_range(0, SEQ_NUMBER_MASK - 2, 1));
        assert!(!seq_in_closed_range(2, SEQ_NUMBER_MASK - 2, 1));
    }

    #[test]
    fn walking_forward_past_the_wrap_stays_consistent() {
        // Start just below the wrap and walk forward 10 steps; sequence
        // order must stay monotonic through the wrap.
        let start = SEQ_NUMBER_MASK - 4;
        let mut prev = start;
        for _ in 0..10 {
            let next = seq_next(prev);
            assert!(seq_lt(prev, next), "prev={prev:#x} next={next:#x}");
            assert_eq!(seq_diff(next, prev), 1);
            prev = next;
        }
    }
}
