//! System Clock Descriptor — ISO/IEC 13818-1 §2.6.20 (tag 0x0B).
//!
//! Describes the system clock accuracy.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_core::len::need_out;
use broadcast_core::reserved::ReservedOnes;
use broadcast_core::{Parse, Serialize};

/// Descriptor tag for system_clock_descriptor.
pub const TAG: u8 = 0x0B;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 2;
/// Default of the 1-bit `reserved` after `external_clock_reference_indicator`
/// (H.222.0 Table 2-64): clause 2.1 "reserved" = `1` (all 9 editions
/// 07/1995-08/2023, audit §5; layout unchanged; confirmed by the TSDuck oracle PMT).
pub const RESERVED_1_DEFAULT: ReservedOnes<RESERVED_AFTER_INDICATOR_BITS> =
    ReservedOnes::<RESERVED_AFTER_INDICATOR_BITS>::FILL;
/// Width of the `reserved` field after the indicator (1 bit).
pub const RESERVED_AFTER_INDICATOR_BITS: u32 = 1;
/// Default of the trailing 5-bit `reserved` (same rule): `11111`.
pub const RESERVED_5_DEFAULT: ReservedOnes<RESERVED_TRAILING_BITS> =
    ReservedOnes::<RESERVED_TRAILING_BITS>::FILL;
/// Width of the trailing `reserved` field (5 bits).
pub const RESERVED_TRAILING_BITS: u32 = 5;
const RESERVED_AFTER_INDICATOR_MASK: u8 = 0x01;
const RESERVED_TRAILING_MASK: u8 = 0x1F;
/// `external_clock_reference_indicator` bit of byte 0.
const EXTERNAL_CLOCK_REFERENCE_INDICATOR_MASK: u8 = 0x80;
/// Shift of `external_clock_reference_indicator` in byte 0.
const EXTERNAL_CLOCK_REFERENCE_INDICATOR_SHIFT: u8 = 7;
/// Mask of the 6-bit `clock_accuracy_integer`.
const CLOCK_ACCURACY_INTEGER_MASK: u8 = 0x3F;
/// Shift of `reserved(1)` in byte 0.
const RESERVED_1_SHIFT: u8 = 6;
/// Shift of `clock_accuracy_exponent` in byte 1.
const CLOCK_ACCURACY_EXPONENT_SHIFT: u8 = 5;
/// Mask of the 3-bit `clock_accuracy_exponent`.
const CLOCK_ACCURACY_EXPONENT_MASK: u8 = 0x07;

/// System Clock Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct SystemClockDescriptor {
    /// External clock reference indicator.
    pub external_clock_reference_indicator: bool,
    /// Clock accuracy integer (6 bits).
    pub clock_accuracy_integer: u8,
    /// Clock accuracy exponent (3 bits).
    pub clock_accuracy_exponent: u8,
    /// `reserved(1)` after the indicator (1 bit). Captured on parse, re-emitted
    /// verbatim; [`SystemClockDescriptor::new`] uses [`RESERVED_1_DEFAULT`].
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_1: ReservedOnes<RESERVED_AFTER_INDICATOR_BITS>,
    /// Trailing `reserved(5)` (5 bits). Captured on parse, re-emitted verbatim;
    /// [`SystemClockDescriptor::new`] uses [`RESERVED_5_DEFAULT`].
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_5: ReservedOnes<RESERVED_TRAILING_BITS>,
}

impl SystemClockDescriptor {
    /// Build with the spec-default reserved bits.
    #[must_use]
    pub const fn new(
        external_clock_reference_indicator: bool,
        clock_accuracy_integer: u8,
        clock_accuracy_exponent: u8,
    ) -> Self {
        Self {
            external_clock_reference_indicator,
            clock_accuracy_integer,
            clock_accuracy_exponent,
            reserved_1: RESERVED_1_DEFAULT,
            reserved_5: RESERVED_5_DEFAULT,
        }
    }
}

impl<'a> Parse<'a> for SystemClockDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SystemClockDescriptor",
            "unexpected tag for system_clock_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "system_clock_descriptor length must equal 2",
            });
        }
        let b0 = body[0];
        let external_clock_reference_indicator =
            (b0 & EXTERNAL_CLOCK_REFERENCE_INDICATOR_MASK) != 0;
        let clock_accuracy_integer = b0 & CLOCK_ACCURACY_INTEGER_MASK;
        let b1 = body[1];
        let clock_accuracy_exponent = b1 >> CLOCK_ACCURACY_EXPONENT_SHIFT;
        Ok(Self {
            external_clock_reference_indicator,
            clock_accuracy_integer,
            clock_accuracy_exponent,
            reserved_1: ReservedOnes::from_bits(u64::from(
                (b0 >> RESERVED_1_SHIFT) & RESERVED_AFTER_INDICATOR_MASK,
            )),
            reserved_5: ReservedOnes::from_bits(u64::from(b1 & RESERVED_TRAILING_MASK)),
        })
    }
}

impl Serialize for SystemClockDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN] = ((self.external_clock_reference_indicator as u8)
            << EXTERNAL_CLOCK_REFERENCE_INDICATOR_SHIFT)
            | ((self.reserved_1.get() as u8) << RESERVED_1_SHIFT)
            | broadcast_core::len::fit_mask(
                self.clock_accuracy_integer,
                CLOCK_ACCURACY_INTEGER_MASK,
                "clock_accuracy_integer",
            )?;
        buf[HEADER_LEN + 1] = (broadcast_core::len::fit_mask(
            self.clock_accuracy_exponent,
            CLOCK_ACCURACY_EXPONENT_MASK,
            "clock_accuracy_exponent",
        )? << CLOCK_ACCURACY_EXPONENT_SHIFT)
            | (self.reserved_5.get() as u8);
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for SystemClockDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SYSTEM_CLOCK";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            2,
            0b1011_1111, // external=1, reserved=0, integer=63
            0b101_00000, // exponent=5, reserved=0
        ];
        let d = SystemClockDescriptor::parse(&bytes).unwrap();
        assert!(d.external_clock_reference_indicator);
        assert_eq!(d.clock_accuracy_integer, 63);
        assert_eq!(d.clock_accuracy_exponent, 5);
    }

    #[test]
    fn serialize_round_trip() {
        let d = SystemClockDescriptor::new(false, 42, 2);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = SystemClockDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = SystemClockDescriptor::parse(&[0x0C, 2, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x0C, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = SystemClockDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = SystemClockDescriptor::new(false, 0, 0);
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall(..)));
    }

    /// H.222.0 Table 2-64 (ISO/IEC 13818-1:2021). Evidence for reserved=1 is the
    /// TSDuck-compiled oracle PMT `tsduck-mpeg-simple-pmt.bin`. Parsed bytes are
    /// preserved verbatim now (a zero-reserved sender is no longer normalized);
    /// only a default-constructed value carries the spec fill.
    #[test]
    fn system_clock_reserved_preserved() {
        for r1 in 0u8..=1 {
            for r5 in [0u8, 1, 0x0A, 0x15, 0x1F] {
                let bytes = [TAG, 2, 0x80 | (r1 << 6) | 0x3F, 0xA0 | r5];
                let d = SystemClockDescriptor::parse(&bytes).unwrap();
                assert_eq!(
                    (d.reserved_1, d.reserved_5),
                    (
                        ReservedOnes::from_bits(u64::from(r1)),
                        ReservedOnes::from_bits(u64::from(r5))
                    )
                );
                let mut out = vec![0u8; d.serialized_len()];
                d.serialize_into(&mut out).unwrap();
                assert_eq!(out, bytes);
            }
        }
    }

    #[test]
    fn system_clock_default_is_spec_value() {
        let d = SystemClockDescriptor::new(true, 63, 5);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 2, 0xFF, 0xBF]);
        let dz = SystemClockDescriptor::parse(&[TAG, 2, 0xBF, 0xA0]).unwrap();
        assert_eq!(dz.reserved_1, ReservedOnes::from_bits(0));
        assert_ne!(dz, d);
    }

    /// Each reserved field is width-limited by its type, so a wider value is
    /// unrepresentable and cannot spill into the adjacent accuracy bits.
    #[test]
    fn reserved_fields_cannot_hold_more_than_their_width() {
        let d = SystemClockDescriptor {
            reserved_1: ReservedOnes::from_bits(0xFF),
            reserved_5: ReservedOnes::from_bits(0xFF),
            ..SystemClockDescriptor::new(false, 0, 0)
        };
        assert_eq!(d.reserved_1.get(), 0x01);
        assert_eq!(d.reserved_5.get(), 0x1F);
    }
}
