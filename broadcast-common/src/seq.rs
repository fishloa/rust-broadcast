//! Wrap-safe modular sequence-number arithmetic (comparable to RFC 1982
//! serial-number arithmetic) over a `bits`-wide sequence space.
//!
//! The one implementation behind `rist-runtime::arq::seq` (16-bit RTP
//! sequence numbers, RFC 3550 §5.1) and `srt-runtime::arq::seq` (31-bit SRT
//! packet sequence numbers, draft-sharabayko-srt-01 §3.1): the two crates
//! carried the same algorithm over different moduli (audit r08-RIST-O1,
//! #1141). Circular over the space; of the two directions between two numbers
//! the shorter one decides "before"/"after". This is implementation policy,
//! not spec-cited — neither protocol specifies a comparison algorithm.

/// Widest supported space: a signed `i32` distance must fit, so 31 bits.
pub const MAX_SEQ_BITS: u32 = 31;

/// A modular sequence-number space of `bits` bits (`1..=31`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqSpace {
    bits: u32,
}

impl SeqSpace {
    /// A `bits`-wide space.
    ///
    /// # Panics
    /// In a `const` context (compile error) or at runtime if `bits` is `0`
    /// or exceeds [`MAX_SEQ_BITS`].
    #[must_use]
    pub const fn new(bits: u32) -> Self {
        assert!(
            bits >= 1 && bits <= MAX_SEQ_BITS,
            "SeqSpace bits out of range"
        );
        Self { bits }
    }

    /// Number of distinct sequence numbers: `2^bits`.
    #[must_use]
    pub const fn modulus(self) -> u64 {
        1u64 << self.bits
    }

    /// Bit mask selecting a sequence number: `2^bits - 1`.
    #[must_use]
    pub const fn mask(self) -> u32 {
        (self.modulus() - 1) as u32
    }

    /// `seq + n`, wrapping at the space boundary.
    #[must_use]
    pub const fn add(self, seq: u32, n: u32) -> u32 {
        // The sum of two `u32`s fits a `u64`; reduced modulo 2^bits it fits a `u32`.
        ((seq as u64 + n as u64) % self.modulus()) as u32
    }

    /// Signed circular distance `a - b`, in `(-half, half]` where `half` is
    /// half the space. Positive means `a` is ahead of `b`. Inputs are masked
    /// to the space.
    #[must_use]
    pub const fn diff(self, a: u32, b: u32) -> i32 {
        let modulus = self.modulus() as i64;
        let half = modulus / 2;
        let raw = ((a & self.mask()) as i64 - (b & self.mask()) as i64).rem_euclid(modulus);
        // Either branch lands in `(-2^30, 2^30]` (bits <= 31), which fits an `i32`.
        (if raw > half { raw - modulus } else { raw }) as i32
    }

    /// `a` precedes `b` in circular order.
    #[must_use]
    pub const fn lt(self, a: u32, b: u32) -> bool {
        self.diff(a, b) < 0
    }

    /// `a` precedes or equals `b`.
    #[must_use]
    pub const fn leq(self, a: u32, b: u32) -> bool {
        self.diff(a, b) <= 0
    }

    /// `a` follows `b` in circular order.
    #[must_use]
    pub const fn gt(self, a: u32, b: u32) -> bool {
        self.diff(a, b) > 0
    }

    /// `a` follows or equals `b`.
    #[must_use]
    pub const fn geq(self, a: u32, b: u32) -> bool {
        self.diff(a, b) >= 0
    }

    /// `seq` lies within the inclusive circular range `first..=last`,
    /// walking forward from `first`. A `last` that precedes `first` in
    /// circular order is an empty (malformed) range and never matches.
    #[must_use]
    pub const fn in_closed_range(self, seq: u32, first: u32, last: u32) -> bool {
        let span = self.diff(last, first);
        if span < 0 {
            return false;
        }
        let offset = self.diff(seq, first);
        offset >= 0 && offset <= span
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S16: SeqSpace = SeqSpace::new(16);
    const S31: SeqSpace = SeqSpace::new(31);

    #[test]
    fn wraps_at_the_space_boundary() {
        assert_eq!(S16.add(0xFFFF, 1), 0);
        assert_eq!(S16.add(0xFFFF, 5), 4);
        assert_eq!(S31.add(0x7FFF_FFFF, 1), 0);
        assert_eq!(S31.add(u32::MAX, u32::MAX), 0x7FFF_FFFE);
    }

    #[test]
    fn ordering_is_circular_and_antisymmetric() {
        assert!(S16.lt(0xFFFF, 0) && S16.gt(0, 0xFFFF));
        assert_eq!(S16.diff(0, 0xFFFF), 1);
        assert!(S31.lt(0x7FFF_FFFF, 0) && S31.geq(5, 5) && S31.leq(5, 5));
        for (a, b) in [(0u32, 0u32), (5, 100), (0xFFFF, 3), (12345, 12340)] {
            assert_eq!(S16.diff(a, b), -S16.diff(b, a));
        }
        // Half the space is the tie point: positive by convention.
        assert_eq!(S16.diff(0x8000, 0), 0x8000);
        assert_eq!(S16.diff(0, 0x8000), 0x8000);
    }

    #[test]
    fn closed_range_walks_forward_across_the_wrap() {
        assert!(S16.in_closed_range(0xFFFE, 0xFFFD, 1));
        assert!(S16.in_closed_range(0, 0xFFFD, 1));
        assert!(!S16.in_closed_range(2, 0xFFFD, 1));
        // `last` before `first` is an empty range.
        assert!(!S16.in_closed_range(5, 10, 3));
    }

    /// Before/after pin (#1141): the shared 16-bit diff equals the formula
    /// `rist-runtime` carried before consolidation, over a strided sweep.
    #[test]
    fn sixteen_bit_diff_matches_the_pre_consolidation_formula() {
        fn reference(a: u16, b: u16) -> i32 {
            let raw = (i32::from(a) - i32::from(b)).rem_euclid(1 << 16);
            if raw > 1 << 15 { raw - (1 << 16) } else { raw }
        }
        for a in (0..=u16::MAX).step_by(257) {
            for b in (0..=u16::MAX).step_by(251) {
                assert_eq!(S16.diff(u32::from(a), u32::from(b)), reference(a, b));
            }
        }
    }

    #[test]
    fn diff_masks_out_of_range_inputs() {
        // SRT's header word carries the F flag in bit 31.
        assert_eq!(S31.diff(0x8000_0001, 0), 1);
    }
}
