//! Wrap-safe 16-bit RTP sequence-number arithmetic.
//!
//! RTP sequence numbers (RFC 3550 §5.1, carried unchanged into TR-06-1
//! Simple Profile — TR-06-2's 32-bit extended sequence number, §8.3, is out
//! of scope; see the `arq` module doc) are a 16-bit field. TR-06-1 does not
//! specify a sequence-number comparison algorithm for ARQ purposes — this
//! module resolves that gap the standard way for a modular sequence space
//! (comparable to RFC 1982 serial number arithmetic: circular over the
//! 16-bit space, picking the shorter of the two directions between two
//! numbers as "before"/"after"). This is **implementation policy**, not
//! spec-cited — needed so the reorder/reassembly buffers behave correctly
//! once a stream's sequence numbers wrap past `0xFFFF` back to `0`.
//!
//! Mirrors `srt_runtime::arq::seq`'s resolution of the same gap for SRT's
//! 31-bit sequence space.

/// Size of the RTP sequence-number space: `2^16` (RFC 3550 §5.1).
const SEQ_MOD: i32 = 1 << 16;
/// Half the sequence space — the wrap-around threshold used to decide which
/// of two directions between two sequence numbers is the shorter one.
const SEQ_HALF: i32 = SEQ_MOD / 2;

/// Add `n` to a sequence number, wrapping at the 16-bit boundary.
pub fn seq_add(seq: u16, n: u16) -> u16 {
    seq.wrapping_add(n)
}

/// The next sequence number after `seq` (wraps `0xFFFF` -> `0`).
pub fn seq_next(seq: u16) -> u16 {
    seq_add(seq, 1)
}

/// Signed circular distance `a - b` in the 16-bit sequence space, in
/// `(-SEQ_HALF, SEQ_HALF]`. Positive means `a` is ahead of `b`.
pub fn seq_diff(a: u16, b: u16) -> i32 {
    let raw = (i32::from(a) - i32::from(b)).rem_euclid(SEQ_MOD);
    if raw > SEQ_HALF { raw - SEQ_MOD } else { raw }
}

/// `a` precedes `b` in circular sequence order.
pub fn seq_lt(a: u16, b: u16) -> bool {
    seq_diff(a, b) < 0
}

/// `a` precedes or equals `b`.
pub fn seq_leq(a: u16, b: u16) -> bool {
    seq_diff(a, b) <= 0
}

/// `a` follows `b` in circular sequence order.
pub fn seq_gt(a: u16, b: u16) -> bool {
    seq_diff(a, b) > 0
}

/// `a` follows or equals `b`.
pub fn seq_geq(a: u16, b: u16) -> bool {
    seq_diff(a, b) >= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_boundary_increment() {
        assert_eq!(seq_next(0xFFFF), 0);
        assert_eq!(seq_add(0xFFFF, 1), 0);
        assert_eq!(seq_add(0xFFFF, 5), 4);
        assert_eq!(seq_add(0, 0xFFFF), 0xFFFF);
    }

    #[test]
    fn wrap_boundary_ordering() {
        assert!(seq_lt(0xFFFF, 0));
        assert!(seq_gt(0, 0xFFFF));
        assert_eq!(seq_diff(0, 0xFFFF), 1);
        assert_eq!(seq_diff(0xFFFF, 0), -1);
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
        for (a, b) in [(0u16, 0u16), (5, 100), (0xFFFF, 3), (12345, 12340)] {
            assert_eq!(seq_diff(a, b), -seq_diff(b, a));
        }
    }

    #[test]
    fn walking_forward_past_the_wrap_stays_consistent() {
        let start: u16 = 0xFFFF - 4;
        let mut prev = start;
        for _ in 0..10 {
            let next = seq_next(prev);
            assert!(seq_lt(prev, next), "prev={prev:#x} next={next:#x}");
            assert_eq!(seq_diff(next, prev), 1);
            prev = next;
        }
    }
}
