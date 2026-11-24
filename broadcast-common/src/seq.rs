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

/// A sequence space wider than `1..=MAX_SEQ_BITS` cannot exist.
///
/// No signed distance fits a space of 32 bits or more (two numbers could be
/// more than `i32::MAX` apart), and a space of no bits has no number in it at
/// all. Reported by [`SeqSpace::try_new`]; there is no panicking constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("sequence space of {bits} bits is outside 1..={MAX_SEQ_BITS}")]
pub struct InvalidSeqSpace {
    /// The width that was asked for.
    pub bits: u32,
}

/// A sequence space of a literal width the compiler can prove in range.
///
/// The whole check happens at compile time: the `match` sits in an inline
/// `const` block, so `seq_space!(32)` is a compile error even inside a `fn`
/// and there is no runtime path that could reach the `panic!`. A runtime
/// width goes through [`SeqSpace::try_new`].
///
/// ```compile_fail
/// // A 32-bit space has no representable signed distance, not even in a fn.
/// fn wide() -> broadcast_common::seq::SeqSpace {
///     broadcast_common::seq_space!(32)
/// }
/// ```
///
/// ```
/// let s = broadcast_common::seq_space!(16);
/// assert_eq!(s.bits(), 16);
/// assert_eq!(s.modulus(), 65_536);
/// ```
#[macro_export]
macro_rules! seq_space {
    ($value:expr) => {
        const {
            match $value {
                bits if bits >= 1 && bits <= $crate::seq::MAX_SEQ_BITS => {
                    $crate::seq::SeqSpace::from_valid_bits(bits)
                }
                _ => panic!("a sequence space is 1..=MAX_SEQ_BITS bits wide"),
            }
        }
    };
}

/// A modular sequence-number space of `bits` bits (`1..=31`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqSpace {
    bits: u32,
}

impl SeqSpace {
    /// A `bits`-wide space.
    ///
    /// # Errors
    /// [`InvalidSeqSpace`] when `bits` is `0` or exceeds [`MAX_SEQ_BITS`];
    /// the width is reported unchanged, never clamped to a space the caller
    /// did not ask for.
    pub const fn try_new(bits: u32) -> Result<Self, InvalidSeqSpace> {
        if bits >= 1 && bits <= MAX_SEQ_BITS {
            Ok(Self { bits })
        } else {
            Err(InvalidSeqSpace { bits })
        }
    }

    /// A `bits`-wide space from a width already proven in range.
    ///
    /// A literal belongs in [`seq_space!`](crate::seq_space!), which proves it
    /// in range at compile time; a runtime width goes through
    /// [`Self::try_new`], which reports the out-of-range one instead.
    #[must_use]
    pub const fn from_valid_bits(bits: u32) -> Self {
        Self { bits }
    }

    /// The width of the space, in bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.bits
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
    /// to the space. At exactly half the space apart the result is `+half` for
    /// both orders; the predicates treat that as no ordering at all.
    #[must_use]
    pub const fn diff(self, a: u32, b: u32) -> i32 {
        let modulus = self.modulus() as i64;
        let half = modulus >> 1;
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

    /// `a` follows `b` in circular order. Exactly half the space apart
    /// neither follows nor precedes the other (see [`SeqSpace::is_tie`]).
    #[must_use]
    pub const fn gt(self, a: u32, b: u32) -> bool {
        self.diff(a, b) > 0 && !self.is_tie(a, b)
    }

    /// `a` follows or equals `b`; `false` at the half-space tie.
    #[must_use]
    pub const fn geq(self, a: u32, b: u32) -> bool {
        self.diff(a, b) >= 0 && !self.is_tie(a, b)
    }

    /// `a` and `b` are exactly half the space apart, so the shorter direction
    /// is undefined: [`diff`](Self::diff) reports `+half` for both orders, and
    /// `lt`/`leq`/`gt`/`geq` all answer `false` so ordering stays antisymmetric.
    #[must_use]
    pub const fn is_tie(self, a: u32, b: u32) -> bool {
        let half = (self.modulus() / 2) as i64;
        ((a & self.mask()) as i64 - (b & self.mask()) as i64).rem_euclid(self.modulus() as i64)
            == half
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
    use alloc::string::ToString;

    const S16: SeqSpace = crate::seq_space!(16);
    const S31: SeqSpace = crate::seq_space!(31);

    /// A width the space cannot hold is a typed error, never a panic a caller
    /// cannot see in the signature or a silently clamped space: `0` has no
    /// number in it, and 32 bits has no representable signed distance.
    #[test]
    fn an_out_of_range_width_is_a_typed_error_not_a_panic() {
        assert_eq!(SeqSpace::try_new(0), Err(InvalidSeqSpace { bits: 0 }));
        assert_eq!(
            SeqSpace::try_new(MAX_SEQ_BITS + 1),
            Err(InvalidSeqSpace {
                bits: MAX_SEQ_BITS + 1
            })
        );
        assert_eq!(
            SeqSpace::try_new(u32::MAX),
            Err(InvalidSeqSpace { bits: u32::MAX })
        );
        assert_eq!(SeqSpace::try_new(1).map(SeqSpace::bits), Ok(1));
        assert_eq!(
            SeqSpace::try_new(MAX_SEQ_BITS).map(SeqSpace::bits),
            Ok(MAX_SEQ_BITS)
        );
        assert_eq!(crate::seq_space!(8).bits(), 8);
        assert_eq!(
            InvalidSeqSpace { bits: 32 }.to_string(),
            "sequence space of 32 bits is outside 1..=31"
        );
    }

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
        assert_eq!(S16.diff(0x8000, 0), 0x8000);
        assert_eq!(S16.diff(0, 0x8000), 0x8000);
        assert!(S16.is_tie(0x8000, 0) && S16.is_tie(0, 0x8000));
    }

    #[test]
    fn ordering_is_antisymmetric_at_exactly_half_the_space() {
        for bits in [8u32, 16, 31] {
            let Some(s) = SeqSpace::try_new(bits).ok() else {
                return;
            };
            let half = (s.modulus() / 2) as u32;
            assert!(!(s.gt(half, 0) && s.gt(0, half)), "bits={bits}");
            assert!(!(s.lt(half, 0) && s.lt(0, half)), "bits={bits}");
            assert!(!s.geq(half, 0) && !s.geq(0, half), "bits={bits}");
            assert!(!s.leq(half, 0) && !s.leq(0, half), "bits={bits}");
            assert!(s.gt(half - 1, 0) && s.lt(0, half - 1), "bits={bits}");
        }
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
