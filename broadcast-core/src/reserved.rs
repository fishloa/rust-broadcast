//! A typed run of `reserved` / `reserved_future_use` / unused bits inside a
//! fixed-width field.
//!
//! Policy (owner decision, mirroring `dvb-si`'s `SectionBits`, #1177): the
//! parser is tolerant — whatever a sender put in a reserved bit field is
//! accepted — and the serializer **writes back what was parsed**, so
//! `parse -> serialize` is byte-exact. A hand-built object defaults to the
//! value the specification fixes for the field (all ones for `reserved`
//! `'1…1'`, zero for a bit the spec says is `0`), which is what
//! [`Default`] yields. The bits are a wire artefact, not part of a decoded
//! view, so a struct carrying one usually marks the field `serde(skip)`.
//!
//! The width is a `const`, so a value wider than the field is unrepresentable
//! and a serializer never has to re-mask one.

/// `BITS` reserved bits (1..=64) whose default is all ones when `ONES` is
/// `true` and all zeros when it is `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Reserved<const BITS: u32, const ONES: bool>(u64);

/// A `BITS`-wide `reserved` field the specification fills with `'1'` bits.
pub type ReservedOnes<const BITS: u32> = Reserved<BITS, true>;
/// A `BITS`-wide field the specification fixes at `'0'` bits.
pub type ReservedZero<const BITS: u32> = Reserved<BITS, false>;

impl<const BITS: u32, const ONES: bool> Reserved<BITS, ONES> {
    /// Mask of the field's `BITS` bits.
    pub const MASK: u64 = {
        assert!(BITS >= 1 && BITS <= 64, "Reserved is 1..=64 bits wide");
        u64::MAX >> (u64::BITS - BITS)
    };

    /// The spec fill: all ones or all zeros.
    pub const FILL: Self = Self(if ONES { Self::MASK } else { 0 });

    /// The field's value taken from the low `BITS` bits of `value` (any
    /// higher bits are not part of the field and are dropped).
    #[must_use]
    pub const fn from_bits(value: u64) -> Self {
        Self(value & Self::MASK)
    }

    /// The field value, right-aligned (`0..=MASK`).
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<const BITS: u32, const ONES: bool> Default for Reserved<BITS, ONES> {
    fn default() -> Self {
        Self::FILL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_spec_fill_and_values_are_masked() {
        assert_eq!(ReservedOnes::<3>::default().get(), 0b111);
        assert_eq!(ReservedOnes::<8>::default().get(), 0xFF);
        assert_eq!(ReservedZero::<4>::default().get(), 0);
        assert_eq!(ReservedOnes::<2>::from_bits(0xF5).get(), 0b01);
        assert_eq!(ReservedZero::<1>::from_bits(0x03).get(), 1);
    }
    #[test]
    fn a_wide_field_holds_its_full_width() {
        assert_eq!(ReservedOnes::<12>::FILL.get(), 0x0FFF);
        assert_eq!(ReservedOnes::<12>::from_bits(0x1_2345).get(), 0x345);
        assert_eq!(ReservedZero::<64>::from_bits(u64::MAX).get(), u64::MAX);
    }
}
