//! STD Descriptor — ISO/IEC 13818-1 §2.6.32 (tag 0x11).
//!
//! Indicates whether the leak method of buffer management is valid.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::len::need_out;
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for STD_descriptor.
pub const TAG: u8 = 0x11;
const HEADER_LEN: usize = 2;
/// Default of the 7-bit `reserved` ahead of `leak_valid_flag` (H.222.0 Table
/// 2-70): clause 2.1 "reserved" = `1` (all 9 editions 07/1995-08/2023, audit
/// §5; confirmed by the TSDuck oracle PMT).
pub const RESERVED_DEFAULT: ReservedOnes<RESERVED_BITS> = ReservedOnes::<RESERVED_BITS>::FILL;
/// Width of the `reserved` field ahead of `leak_valid_flag` (7 bits).
pub const RESERVED_BITS: u32 = 7;
/// `leak_valid_flag` occupies the low bit of the body byte.
const LEAK_VALID_FLAG_MASK: u8 = 0x01;
/// `reserved` starts at bit 1 of the body byte.
const RESERVED_SHIFT: u32 = 1;
const BODY_LEN: u8 = 1;

/// STD Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct StdDescriptor {
    /// Leak valid flag — 1 means the leak method is applicable.
    pub leak_valid_flag: bool,
    /// `reserved(7)` ahead of the flag (7 bits). Captured on parse,
    /// re-emitted verbatim; [`StdDescriptor::new`] uses [`RESERVED_DEFAULT`].
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<RESERVED_BITS>,
}

impl StdDescriptor {
    /// Build with the spec-default reserved bits.
    #[must_use]
    pub const fn new(leak_valid_flag: bool) -> Self {
        Self {
            leak_valid_flag,
            reserved: RESERVED_DEFAULT,
        }
    }
}

impl<'a> Parse<'a> for StdDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "StdDescriptor",
            "unexpected tag for STD_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "STD_descriptor length must equal 1",
            });
        }
        let leak_valid_flag = (body[0] & LEAK_VALID_FLAG_MASK) != 0;
        Ok(Self {
            leak_valid_flag,
            reserved: ReservedOnes::from_bits(u64::from(body[0] >> RESERVED_SHIFT)),
        })
    }
}

impl Serialize for StdDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN] = ((self.reserved.get() as u8) << 1) | self.leak_valid_flag as u8;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for StdDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "STD";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse_leak_valid() {
        let bytes = [TAG, 1, 0b0000_0001];
        let d = StdDescriptor::parse(&bytes).unwrap();
        assert!(d.leak_valid_flag);
    }

    #[test]
    fn parse_leak_not_valid() {
        let bytes = [TAG, 1, 0b0000_0000];
        let d = StdDescriptor::parse(&bytes).unwrap();
        assert!(!d.leak_valid_flag);
    }

    #[test]
    fn serialize_round_trip() {
        let d = StdDescriptor::new(true);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = StdDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = StdDescriptor::parse(&[0x12, 1, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x12, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = StdDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = StdDescriptor::new(false);
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall(..)));
    }

    /// H.222.0 Table 2-70. Parsed reserved bits are preserved (zero-reserved
    /// senders are no longer normalized to ones); defaults carry the spec fill.
    #[test]
    fn std_reserved_preserved() {
        for r in 0u8..=0x7F {
            for flag in 0u8..=1 {
                let bytes = [TAG, 1, (r << 1) | flag];
                let d = StdDescriptor::parse(&bytes).unwrap();
                assert_eq!(d.reserved, ReservedOnes::from_bits(u64::from(r)));
                let mut out = vec![0u8; d.serialized_len()];
                d.serialize_into(&mut out).unwrap();
                assert_eq!(out, bytes);
            }
        }
    }

    #[test]
    fn std_default_is_spec_value() {
        let d = StdDescriptor::new(true);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 1, 0xFF]);
    }

    /// The field is 7 bits wide, so a wider value is unrepresentable and can
    /// never leak into the flag bit below it.
    #[test]
    fn reserved_cannot_hold_more_than_seven_bits() {
        let d = StdDescriptor {
            leak_valid_flag: false,
            reserved: ReservedOnes::from_bits(0b1111_1111),
        };
        assert_eq!(d.reserved.get(), 0x7F);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 1, 0xFE]);
    }
}
