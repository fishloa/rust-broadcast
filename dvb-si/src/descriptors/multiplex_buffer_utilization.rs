//! Multiplex Buffer Utilization Descriptor — ISO/IEC 13818-1 §2.6.22 (tag 0x0C).
//!
//! Bound on the portion of the multiplex buffer occupied without
//! overflow/underflow of the STD.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::len::need_out;
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for multiplex_buffer_utilization_descriptor.
pub const TAG: u8 = 0x0C;
const HEADER_LEN: usize = 2;
/// Default of the 1-bit `reserved` ahead of `LTW_offset_upper_bound` (H.222.0
/// Table 2-65): clause 2.1 "reserved" = `1` (all 9 editions 07/1995-08/2023,
/// audit §5; no fixture carries the descriptor).
pub const RESERVED_DEFAULT: ReservedOnes<RESERVED_BITS> = ReservedOnes::<RESERVED_BITS>::FILL;
/// Width of the `reserved` field ahead of the upper bound (1 bit).
const RESERVED_BITS: u32 = 1;
const BODY_LEN: u8 = 4;
/// `bound_valid_flag` bit of the first body byte.
const BOUND_VALID_FLAG: u8 = 0x80;
/// Width of each 15-bit LTW offset field.
const BOUND_FIELD_BITS: u32 = 15;
/// Mask of each 15-bit LTW offset field.
const BOUND_FIELD_MASK: u16 = (1 << BOUND_FIELD_BITS) - 1;
/// Shift of the high LTW offset bits within a 16-bit packed value.
const BOUND_HI_SHIFT: u16 = 8;
/// Body offset of `LTW_offset_upper_bound`.
const UPPER_BOUND_OFFSET: usize = 2;
/// Shift of the 1-bit `reserved` run in the upper-bound high byte.
const RESERVED_SHIFT: u16 = 7;

/// Multiplex Buffer Utilization Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct MultiplexBufferUtilizationDescriptor {
    /// Bound valid flag.
    pub bound_valid_flag: bool,
    /// LTW offset lower bound (15 bits).
    pub ltw_offset_lower_bound: u16,
    /// LTW offset upper bound (15 bits).
    pub ltw_offset_upper_bound: u16,
    /// `reserved(1)` ahead of the upper bound (1 bit). Captured on parse,
    /// re-emitted verbatim; [`MultiplexBufferUtilizationDescriptor::new`] uses
    /// [`RESERVED_DEFAULT`].
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<RESERVED_BITS>,
}

impl MultiplexBufferUtilizationDescriptor {
    /// Build with the spec-default reserved bit.
    #[must_use]
    pub const fn new(
        bound_valid_flag: bool,
        ltw_offset_lower_bound: u16,
        ltw_offset_upper_bound: u16,
    ) -> Self {
        Self {
            bound_valid_flag,
            ltw_offset_lower_bound,
            ltw_offset_upper_bound,
            reserved: RESERVED_DEFAULT,
        }
    }
}

impl<'a> Parse<'a> for MultiplexBufferUtilizationDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MultiplexBufferUtilizationDescriptor",
            "unexpected tag for multiplex_buffer_utilization_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "multiplex_buffer_utilization_descriptor length must equal 4",
            });
        }
        let bound_valid_flag = (body[0] & BOUND_VALID_FLAG) != 0;
        let ltw_offset_lower_bound =
            ((u16::from(body[0]) & BOUND_FIELD_MASK) << BOUND_HI_SHIFT) | u16::from(body[1]);
        let ltw_offset_upper_bound = ((u16::from(body[UPPER_BOUND_OFFSET]) & BOUND_FIELD_MASK)
            << BOUND_HI_SHIFT)
            | u16::from(body[UPPER_BOUND_OFFSET + 1]);
        Ok(Self {
            bound_valid_flag,
            ltw_offset_lower_bound,
            ltw_offset_upper_bound,
            reserved: ReservedOnes::from_bits(u64::from(body[UPPER_BOUND_OFFSET] >> RESERVED_SHIFT)),
        })
    }
}

impl Serialize for MultiplexBufferUtilizationDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        // 15-bit LTW offsets; a wider value used to be masked down silently
        // (audit r03-W3 class).
        broadcast_common::len::fit_bits(
            u64::from(self.ltw_offset_lower_bound),
            BOUND_FIELD_BITS,
            "ltw_offset_lower_bound",
        )?;
        broadcast_common::len::fit_bits(
            u64::from(self.ltw_offset_upper_bound),
            BOUND_FIELD_BITS,
            "ltw_offset_upper_bound",
        )?;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        let lower =
            (u32::from(self.bound_valid_flag as u8) << BOUND_FIELD_BITS)
                | u32::from(self.ltw_offset_lower_bound);
        buf[HEADER_LEN..HEADER_LEN + size_of::<u16>()].copy_from_slice(&lower.to_be_bytes()[2..]);
        let upper = (u32::from(self.reserved.get() as u8) << BOUND_FIELD_BITS)
            | u32::from(self.ltw_offset_upper_bound);
        buf[HEADER_LEN + UPPER_BOUND_OFFSET..HEADER_LEN + UPPER_BOUND_OFFSET + size_of::<u16>()]
            .copy_from_slice(&upper.to_be_bytes()[2..]);
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for MultiplexBufferUtilizationDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MULTIPLEX_BUFFER_UTILIZATION";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            4,
            0b1_1111111,
            0xAB, // bound_valid=1, lower_bound=0x7FAB
            0b1_1111111,
            0xCD, // reserved(1) in source, upper_bound=0x7FCD
        ];
        let d = MultiplexBufferUtilizationDescriptor::parse(&bytes).unwrap();
        assert!(d.bound_valid_flag);
        assert_eq!(d.ltw_offset_lower_bound, 0x7FAB);
        assert_eq!(d.ltw_offset_upper_bound, 0x7FCD);
    }

    #[test]
    fn serialize_round_trip() {
        let d = MultiplexBufferUtilizationDescriptor::new(false, 0x1234, 0x5678);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MultiplexBufferUtilizationDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MultiplexBufferUtilizationDescriptor::parse(&[0x0D, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x0D, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = MultiplexBufferUtilizationDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = MultiplexBufferUtilizationDescriptor::new(false, 0, 0);
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall(..)));
    }

    /// H.222.0 Table 2-65. Parsed reserved bit is preserved (no longer
    /// normalized to 1); defaults carry the spec fill.
    #[test]
    fn multiplex_buffer_utilization_reserved_preserved() {
        for r in 0u8..=1 {
            let bytes = [TAG, 4, 0x12, 0x34, (r << 7) | 0x56, 0x78];
            let d = MultiplexBufferUtilizationDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved, ReservedOnes::from_bits(u64::from(r)));
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn multiplex_buffer_utilization_default_is_spec_value() {
        let d = MultiplexBufferUtilizationDescriptor::new(false, 0x1234, 0x5678);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 4, 0x12, 0x34, 0xD6, 0x78]);
    }

    /// The field is 1 bit wide, so a wider value is unrepresentable and cannot
    /// spill into the 15-bit upper bound below it.
    #[test]
    fn reserved_cannot_hold_more_than_one_bit() {
        let d = MultiplexBufferUtilizationDescriptor {
            bound_valid_flag: false,
            ltw_offset_lower_bound: 0x1234,
            ltw_offset_upper_bound: 0x5678,
            reserved: ReservedOnes::from_bits(0xFF),
        };
        assert_eq!(d.reserved.get(), 0x01);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 4, 0x12, 0x34, 0xD6, 0x78]);
    }
}
