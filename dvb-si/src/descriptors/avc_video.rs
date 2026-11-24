//! AVC Video Descriptor — ISO/IEC 13818-1 §2.6.64 (tag 0x28).
//!
//! Describes the profile, level, and constraint flags for an AVC/H.264
//! video elementary stream.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::len::need_out;
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for AVC_video_descriptor.
pub const TAG: u8 = 0x28;
const HEADER_LEN: usize = 2;
/// Width of the `reserved` tail of body byte 3 (5 bits).
const RESERVED_BITS: u32 = 5;
/// Default of the 5-bit `reserved` tail of body byte 3 (H.222.0 Table 2-105):
/// clause 2.1 "reserved" = `1` (all editions that carry the descriptor,
/// 06/2012-08/2023, audit §4 row 8 / §5; the 2012 edition carved
/// `Frame_Packing_SEI_not_present_flag` out of it, which is typed here).
pub const RESERVED_DEFAULT: ReservedOnes<RESERVED_BITS> = ReservedOnes::<RESERVED_BITS>::FILL;
const BODY_LEN: u8 = 4;
/// Offset of the constraint-flag byte in the descriptor body.
const CONSTRAINT_FLAGS_OFFSET: usize = 1;
/// Offset of `level_idc` in the descriptor body.
const LEVEL_IDC_OFFSET: usize = 2;
/// Offset of the still-present/24-hour/frame-packing tail byte.
const TAIL_FLAGS_OFFSET: usize = 3;
const CONSTRAINT_SET0_FLAG_MASK: u8 = 0x80;
const CONSTRAINT_SET1_FLAG_MASK: u8 = 0x40;
const CONSTRAINT_SET2_FLAG_MASK: u8 = 0x20;
const CONSTRAINT_SET3_FLAG_MASK: u8 = 0x10;
const CONSTRAINT_SET4_FLAG_MASK: u8 = 0x08;
const CONSTRAINT_SET5_FLAG_MASK: u8 = 0x04;
const AVC_COMPATIBLE_FLAGS_MASK: u8 = 0x03;
const AVC_STILL_PRESENT_MASK: u8 = 0x80;
const AVC_24_HOUR_PICTURE_FLAG_MASK: u8 = 0x40;
const FRAME_PACKING_SEI_NOT_PRESENT_FLAG_MASK: u8 = 0x20;

/// AVC Video Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct AvcVideoDescriptor {
    /// AVC profile_idc.
    pub profile_idc: u8,
    /// Constraint set 0 flag.
    pub constraint_set0_flag: bool,
    /// Constraint set 1 flag.
    pub constraint_set1_flag: bool,
    /// Constraint set 2 flag.
    pub constraint_set2_flag: bool,
    /// Constraint set 3 flag.
    pub constraint_set3_flag: bool,
    /// Constraint set 4 flag.
    pub constraint_set4_flag: bool,
    /// Constraint set 5 flag.
    pub constraint_set5_flag: bool,
    /// AVC compatible flags (2 bits).
    pub avc_compatible_flags: u8,
    /// AVC level_idc.
    pub level_idc: u8,
    /// AVC still present flag.
    pub avc_still_present: bool,
    /// AVC 24 hour picture flag.
    pub avc_24_hour_picture_flag: bool,
    /// Frame packing SEI not present flag.
    pub frame_packing_sei_not_present_flag: bool,
    /// `reserved(5)` tail of body byte 3 (5 bits, right-aligned). Captured on
    /// parse, re-emitted verbatim; [`AvcVideoDescriptor::new`] /
    /// [`Default`] use [`RESERVED_DEFAULT`].
    pub reserved: ReservedOnes<RESERVED_BITS>,
}

impl AvcVideoDescriptor {
    /// Build with the identifying fields; every flag clear, `avc_compatible_flags`
    /// 0 and the spec-default reserved bits. Set the flag fields directly.
    #[must_use]
    pub const fn new(profile_idc: u8, level_idc: u8) -> Self {
        Self {
            profile_idc,
            constraint_set0_flag: false,
            constraint_set1_flag: false,
            constraint_set2_flag: false,
            constraint_set3_flag: false,
            constraint_set4_flag: false,
            constraint_set5_flag: false,
            avc_compatible_flags: 0,
            level_idc,
            avc_still_present: false,
            avc_24_hour_picture_flag: false,
            frame_packing_sei_not_present_flag: false,
            reserved: RESERVED_DEFAULT,
        }
    }
}

impl Default for AvcVideoDescriptor {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl<'a> Parse<'a> for AvcVideoDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AvcVideoDescriptor",
            "unexpected tag for AVC_video_descriptor",
        )?;
        if body.len() != (BODY_LEN as usize) {
            // Table 2-105 fixes the body at 4 bytes. A longer body used to be
            // accepted and its trailing bytes silently dropped on re-serialize,
            // so a non-conformant stream did not round-trip byte-identically
            // (r03-W9).
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "AVC_video_descriptor body length must equal 4",
            });
        }
        let b0 = body[0]; // profile_idc
        let b1 = body[CONSTRAINT_FLAGS_OFFSET]; // constraint_set0_flag..avc_compatible_flags
        let b2 = body[LEVEL_IDC_OFFSET]; // level_idc
        let b3 = body[TAIL_FLAGS_OFFSET]; // avc_still_present..reserved

        Ok(Self {
            profile_idc: b0,
            constraint_set0_flag: (b1 & CONSTRAINT_SET0_FLAG_MASK) != 0,
            constraint_set1_flag: (b1 & CONSTRAINT_SET1_FLAG_MASK) != 0,
            constraint_set2_flag: (b1 & CONSTRAINT_SET2_FLAG_MASK) != 0,
            constraint_set3_flag: (b1 & CONSTRAINT_SET3_FLAG_MASK) != 0,
            constraint_set4_flag: (b1 & CONSTRAINT_SET4_FLAG_MASK) != 0,
            constraint_set5_flag: (b1 & CONSTRAINT_SET5_FLAG_MASK) != 0,
            avc_compatible_flags: b1 & AVC_COMPATIBLE_FLAGS_MASK,
            level_idc: b2,
            avc_still_present: (b3 & AVC_STILL_PRESENT_MASK) != 0,
            avc_24_hour_picture_flag: (b3 & AVC_24_HOUR_PICTURE_FLAG_MASK) != 0,
            frame_packing_sei_not_present_flag: (b3 & FRAME_PACKING_SEI_NOT_PRESENT_FLAG_MASK) != 0,
            reserved: ReservedOnes::from_bits(u64::from(b3)),
        })
    }
}

impl Serialize for AvcVideoDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN] = self.profile_idc;
        buf[HEADER_LEN + CONSTRAINT_FLAGS_OFFSET] =
            (u8::from(self.constraint_set0_flag) * CONSTRAINT_SET0_FLAG_MASK)
                | (u8::from(self.constraint_set1_flag) * CONSTRAINT_SET1_FLAG_MASK)
                | (u8::from(self.constraint_set2_flag) * CONSTRAINT_SET2_FLAG_MASK)
                | (u8::from(self.constraint_set3_flag) * CONSTRAINT_SET3_FLAG_MASK)
                | (u8::from(self.constraint_set4_flag) * CONSTRAINT_SET4_FLAG_MASK)
                | (u8::from(self.constraint_set5_flag) * CONSTRAINT_SET5_FLAG_MASK)
                | broadcast_common::len::fit_mask(
                    self.avc_compatible_flags,
                    AVC_COMPATIBLE_FLAGS_MASK,
                    "avc_compatible_flags",
                )?;
        buf[HEADER_LEN + LEVEL_IDC_OFFSET] = self.level_idc;
        buf[HEADER_LEN + TAIL_FLAGS_OFFSET] =
            (u8::from(self.avc_still_present) * AVC_STILL_PRESENT_MASK)
                | (u8::from(self.avc_24_hour_picture_flag) * AVC_24_HOUR_PICTURE_FLAG_MASK)
                | (u8::from(self.frame_packing_sei_not_present_flag)
                    * FRAME_PACKING_SEI_NOT_PRESENT_FLAG_MASK)
                | self.reserved.get() as u8;
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for AvcVideoDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AVC_VIDEO";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn round_trip() {
        let orig = AvcVideoDescriptor {
            profile_idc: 0x4D,
            constraint_set0_flag: true,
            constraint_set1_flag: false,
            constraint_set2_flag: false,
            constraint_set3_flag: true,
            constraint_set4_flag: false,
            constraint_set5_flag: true,
            avc_compatible_flags: 0x03,
            level_idc: 0x29,
            avc_still_present: false,
            avc_24_hour_picture_flag: true,
            frame_packing_sei_not_present_flag: false,
            reserved: RESERVED_DEFAULT,
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = AvcVideoDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = AvcVideoDescriptor::parse(&[0x02, 4, 0x00, 0x00, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        // length=2 with a present 2-byte body (< BODY_LEN=4) hits the
        // descriptor's own length check → InvalidDescriptor.
        let err = AvcVideoDescriptor::parse(&[TAG, 2, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_rejects_trailing_bytes() {
        // Table 2-105 fixes the body at 4 bytes; the 5th byte has no wire
        // meaning and used to be dropped on re-serialize (r03-W9).
        let err = AvcVideoDescriptor::parse(&[TAG, 5, 0x00, 0x00, 0x00, 0x00, 0xFF]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    /// H.222.0 Table 2-105. Parsed reserved bits are preserved.
    #[test]
    fn avc_video_reserved_preserved() {
        for r in 0u8..=0x1F {
            let bytes = [TAG, 4, 0x4D, 0x95, 0x29, 0x40 | r];
            let d = AvcVideoDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved.get(), u64::from(r));
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    /// A value wider than the 5-bit field is unrepresentable.
    #[test]
    fn avc_video_reserved_is_five_bits() {
        assert_eq!(
            ReservedOnes::<RESERVED_BITS>::from_bits(u64::MAX).get(),
            0x1F
        );
    }

    #[test]
    fn avc_video_default_is_spec_value() {
        let d = AvcVideoDescriptor::new(0x4D, 0x29);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 4, 0x4D, 0x00, 0x29, 0x1F]);
    }
}
