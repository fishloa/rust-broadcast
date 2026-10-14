//! HEVC Video Descriptor — ISO/IEC 13818-1 §2.6.95 (tag 0x38).
//!
//! Describes the profile, tier, level, and constraint flags for an
//! HEVC/H.265 video elementary stream. The `copied_44bits` field is a
//! 44-bit non-byte-aligned region stored as a u64 masked to 44 bits.
//! The temporal_id block is conditional on `temporal_layer_subset_flag`.

use super::descriptor_body;
use super::hdr_wcg_idc::HdrWcgIdc;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for HEVC_video_descriptor.
pub const TAG: u8 = 0x38;
const HEADER_LEN: usize = 2;
const COPIED_44_MASK: u64 = (1 << 44) - 1;
/// `profile_space(2)` (ISO/IEC 23001-2 Table 1); raw field, mask so a wide
/// value cannot bleed into `tier_flag` (r03-W2).
const PROFILE_SPACE_MASK: u8 = 0x03;
/// `profile_idc(5)`.
const PROFILE_IDC_MASK: u8 = 0x1F;
/// `temporal_id_min(3)` / `temporal_id_max(3)`.
const TEMPORAL_ID_MASK: u8 = 0x07;
/// Default of the 2-bit `reserved` at bits `[3:2]` of byte 12 (H.222.0 Table
/// 2-113, editions 03/2017-08/2023, audit §4 row 10 / §9): a single 2-bit
/// `reserved` field, clause 2.1 "reserved" = `1`, so both bits are `1`.
/// Real captures confirm it: every HEVC_video_descriptor in the local archive
/// (9, from TSDuck-compiled and re-muxed encoder output) carries
/// `0x9F`/`0x7B`-style flag bytes with bits `[3:2]` = `11`. (An earlier
/// reading cleared bit 3 on the strength of ISO/IEC 23001-2; that contradicted
/// both the H.222.0 table and the data.) Parse preserves whatever is read.
pub const RESERVED_DEFAULT: u8 = 0b11;
const RESERVED_MASK: u8 = 0x03;
/// Default of the 5-bit `reserved` in each temporal-sub-block byte (same
/// rule): `11111`.
pub const TEMPORAL_RESERVED_DEFAULT: u8 = 0b1_1111;
const TEMPORAL_RESERVED_MASK: u8 = 0x1F;
/// Fixed body length without temporal_id sub-block.
const FIXED_BODY_LEN: u8 = 12;
/// Temporal sub-block length when present.
const TEMPORAL_SUB_LEN: u8 = 2;

/// Temporal layer subset sub-block — present when `temporal_layer_subset_flag` is true.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcTemporalSub {
    /// Minimum temporal_id value.
    pub temporal_id_min: u8,
    /// Maximum temporal_id value.
    pub temporal_id_max: u8,
    /// `reserved(5)` after `temporal_id_min` (5 bits, right-aligned). Captured
    /// on parse, re-emitted verbatim; defaults to
    /// [`TEMPORAL_RESERVED_DEFAULT`].
    pub reserved_min: u8,
    /// `reserved(5)` after `temporal_id_max` (5 bits, right-aligned). Captured
    /// on parse, re-emitted verbatim; defaults to
    /// [`TEMPORAL_RESERVED_DEFAULT`].
    pub reserved_max: u8,
}

impl HevcTemporalSub {
    /// Build with the spec-default reserved bits.
    #[must_use]
    pub const fn new(temporal_id_min: u8, temporal_id_max: u8) -> Self {
        Self {
            temporal_id_min,
            temporal_id_max,
            reserved_min: TEMPORAL_RESERVED_DEFAULT,
            reserved_max: TEMPORAL_RESERVED_DEFAULT,
        }
    }
}

/// HEVC Video Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct HevcVideoDescriptor {
    /// Profile space (2 bits).
    pub profile_space: u8,
    /// Tier flag.
    pub tier_flag: bool,
    /// Profile IDC (5 bits).
    pub profile_idc: u8,
    /// Profile compatibility indication (32 bits).
    pub profile_compatibility_indication: u32,
    /// Progressive source flag.
    pub progressive_source_flag: bool,
    /// Interlaced source flag.
    pub interlaced_source_flag: bool,
    /// Non-packed constraint flag.
    pub non_packed_constraint_flag: bool,
    /// Frame only constraint flag.
    pub frame_only_constraint_flag: bool,
    /// Copied 44 bits — stored as u64, masked to 44 bits.
    pub copied_44bits: u64,
    /// Level IDC.
    pub level_idc: u8,
    /// Temporal layer subset flag.
    pub temporal_layer_subset_flag: bool,
    /// HEVC still present flag.
    pub hevc_still_present_flag: bool,
    /// HEVC 24hr picture present flag.
    pub hevc_24hr_picture_present_flag: bool,
    /// Sub-pic HRD params not present flag.
    pub sub_pic_hrd_params_not_present_flag: bool,
    /// HDR/WCG indication (Table 2-114).
    pub hdr_wcg_idc: HdrWcgIdc,
    /// `reserved(2)` at bits `[3:2]` of byte 12 (2 bits, right-aligned).
    /// Captured on parse, re-emitted verbatim; [`HevcVideoDescriptor::new`] /
    /// [`Default`] use [`RESERVED_DEFAULT`] (`0b11`, the H.222.0 value).
    pub reserved: u8,
    /// Temporal layer sub-block, when present.
    pub temporal_sub: Option<HevcTemporalSub>,
}

impl HevcVideoDescriptor {
    /// Build with the identifying fields; every flag clear, `copied_44bits` 0,
    /// no HDR/WCG indication, no temporal sub-block and the spec-default
    /// reserved bits. Set the remaining fields directly.
    #[must_use]
    pub const fn new(
        profile_space: u8,
        tier_flag: bool,
        profile_idc: u8,
        profile_compatibility_indication: u32,
        level_idc: u8,
    ) -> Self {
        Self {
            profile_space,
            tier_flag,
            profile_idc,
            profile_compatibility_indication,
            progressive_source_flag: false,
            interlaced_source_flag: false,
            non_packed_constraint_flag: false,
            frame_only_constraint_flag: false,
            copied_44bits: 0,
            level_idc,
            temporal_layer_subset_flag: false,
            hevc_still_present_flag: false,
            hevc_24hr_picture_present_flag: false,
            sub_pic_hrd_params_not_present_flag: false,
            hdr_wcg_idc: HdrWcgIdc::NoIndication,
            reserved: RESERVED_DEFAULT,
            temporal_sub: None,
        }
    }
}

impl Default for HevcVideoDescriptor {
    fn default() -> Self {
        Self::new(0, false, 0, 0, 0)
    }
}

impl<'a> Parse<'a> for HevcVideoDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "HevcVideoDescriptor",
            "unexpected tag for HEVC_video_descriptor",
        )?;
        if body.len() < (FIXED_BODY_LEN as usize) {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "HEVC_video_descriptor too short",
            });
        }

        let b0 = body[0]; // profile_space(2)|tier_flag(1)|profile_idc(5)
        let profile_space = b0 >> 6;
        let tier_flag = (b0 & 0x20) != 0;
        let profile_idc = b0 & 0x1F;

        // profile_compatibility_indication: bytes 1..5 (32 bits big-endian)
        let profile_compatibility_indication =
            u32::from_be_bytes([body[1], body[2], body[3], body[4]]);

        let b5 = body[5]; // progressive_source_flag..frame_only_constraint_flag | copied_44bits hi 4 bits
        let progressive_source_flag = (b5 & 0x80) != 0;
        let interlaced_source_flag = (b5 & 0x40) != 0;
        let non_packed_constraint_flag = (b5 & 0x20) != 0;
        let frame_only_constraint_flag = (b5 & 0x10) != 0;

        // copied_44bits: 4 bits from b5 (low nibble) + 5 full bytes (body[6..11]) = 44 bits
        let copied_44bits_hi: u64 = (b5 as u64 & 0x0F) << 40;
        let copied_44bits_lo: u64 = ((body[6] as u64) << 32)
            | ((body[7] as u64) << 24)
            | ((body[8] as u64) << 16)
            | ((body[9] as u64) << 8)
            | (body[10] as u64);
        let copied_44bits = (copied_44bits_hi | copied_44bits_lo) & COPIED_44_MASK;

        let b11 = body[11]; // level_idc

        // Byte 12 is mandatory (ISO/IEC 13818-1 Table 2-113): a 12-byte body
        // used to be accepted with the flags defaulted, and then re-serialized
        // as 13 bytes, so such a stream was neither conformant nor
        // byte-identical (r03-W12).
        if body.len() < (FIXED_BODY_LEN as usize) + 1 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "HEVC_video_descriptor too short for the mandatory flags byte",
            });
        }
        // byte 12: tls(7)|still(6)|24hr(5)|sub_pic(4)|reserved(3:2)|HDR_WCG_idc(1:0)
        let b12 = body[FIXED_BODY_LEN as usize];
        let temporal_layer_subset_flag = (b12 & 0x80) != 0;
        let hevc_still_present_flag = (b12 & 0x40) != 0;
        let hevc_24hr_picture_present_flag = (b12 & 0x20) != 0;
        let sub_pic_hrd_params_not_present_flag = (b12 & 0x10) != 0;
        let hdr_wcg_idc = HdrWcgIdc::from_u8(b12 & 0x03);

        let temporal_sub = if temporal_layer_subset_flag {
            if body.len() != (FIXED_BODY_LEN + 1 + TEMPORAL_SUB_LEN) as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "HEVC_video_descriptor body length disagrees with temporal_layer_subset_flag",
                });
            }
            let b13 = body[13];
            let b14 = body[14];
            Some(HevcTemporalSub {
                temporal_id_min: b13 >> 5,
                temporal_id_max: b14 >> 5,
                reserved_min: b13 & TEMPORAL_RESERVED_MASK,
                reserved_max: b14 & TEMPORAL_RESERVED_MASK,
            })
        } else {
            if body.len() != (FIXED_BODY_LEN as usize) + 1 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "HEVC_video_descriptor has trailing bytes",
                });
            }
            None
        };

        Ok(Self {
            profile_space,
            tier_flag,
            profile_idc,
            profile_compatibility_indication,
            progressive_source_flag,
            interlaced_source_flag,
            non_packed_constraint_flag,
            frame_only_constraint_flag,
            copied_44bits,
            level_idc: b11,
            temporal_layer_subset_flag,
            hevc_still_present_flag,
            hevc_24hr_picture_present_flag,
            sub_pic_hrd_params_not_present_flag,
            hdr_wcg_idc,
            reserved: (b12 >> 2) & RESERVED_MASK,
            temporal_sub,
        })
    }
}

impl Serialize for HevcVideoDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let extra = if self.temporal_layer_subset_flag {
            TEMPORAL_SUB_LEN
        } else {
            0
        };
        HEADER_LEN + (FIXED_BODY_LEN as usize) + 1 + (extra as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let body_len = (len - HEADER_LEN) as u8;
        buf[0] = TAG;
        buf[1] = body_len;

        // byte 0: profile_space(2)|tier_flag(1)|profile_idc(5)
        if self.profile_space > PROFILE_SPACE_MASK {
            return Err(Error::ValueOutOfRange {
                field: "profile_space",
                reason: "exceeds 2-bit field",
            });
        }
        if self.profile_idc > PROFILE_IDC_MASK {
            return Err(Error::ValueOutOfRange {
                field: "profile_idc",
                reason: "exceeds 5-bit field",
            });
        }
        buf[HEADER_LEN] = (broadcast_common::len::fit_mask(
            self.profile_space,
            PROFILE_SPACE_MASK,
            "profile_space",
        )? << 6)
            | ((self.tier_flag as u8) << 5)
            | broadcast_common::len::fit_mask(self.profile_idc, PROFILE_IDC_MASK, "profile_idc")?;

        // bytes 1..5: profile_compatibility_indication (32 bits big-endian)
        buf[HEADER_LEN + 1..HEADER_LEN + 5]
            .copy_from_slice(&self.profile_compatibility_indication.to_be_bytes());

        // byte 5: flags(4) | copied_44bits hi 4 bits
        let copied =
            broadcast_common::len::fit_mask(self.copied_44bits, COPIED_44_MASK, "copied_44bits")?;
        buf[HEADER_LEN + 5] = ((self.progressive_source_flag as u8) << 7)
            | ((self.interlaced_source_flag as u8) << 6)
            | ((self.non_packed_constraint_flag as u8) << 5)
            | ((self.frame_only_constraint_flag as u8) << 4)
            | (((copied >> 40) & 0x0F) as u8);

        // bytes 6..11: copied_44bits lo 40 bits
        buf[HEADER_LEN + 6] = ((copied >> 32) & 0xFF) as u8;
        buf[HEADER_LEN + 7] = ((copied >> 24) & 0xFF) as u8;
        buf[HEADER_LEN + 8] = ((copied >> 16) & 0xFF) as u8;
        buf[HEADER_LEN + 9] = ((copied >> 8) & 0xFF) as u8;
        buf[HEADER_LEN + 10] = (copied & 0xFF) as u8;

        // byte 11: level_idc
        buf[HEADER_LEN + 11] = self.level_idc;

        // byte 12: tls(7)|still(6)|24hr(5)|sub_pic(4)|reserved(3:2)|HDR_WCG_idc(1:0)
        buf[HEADER_LEN + 12] = ((self.temporal_layer_subset_flag as u8) << 7)
            | ((self.hevc_still_present_flag as u8) << 6)
            | ((self.hevc_24hr_picture_present_flag as u8) << 5)
            | ((self.sub_pic_hrd_params_not_present_flag as u8) << 4)
            | (broadcast_common::len::fit_mask(self.reserved, RESERVED_MASK, "reserved")? << 2)
            | broadcast_common::len::fit_mask(self.hdr_wcg_idc.to_u8(), 0x03, "hdr_wcg_idc")?;

        if self.temporal_layer_subset_flag != self.temporal_sub.is_some() {
            // serialized_len reserves the 2 temporal-sub bytes from the flag
            // alone; writing them from the Option too would overrun or leave
            // stale bytes (r03-W6).
            return Err(Error::ValueOutOfRange {
                field: "temporal_sub",
                reason: "presence must match temporal_layer_subset_flag",
            });
        }
        if self.temporal_layer_subset_flag
            && let Some(ref ts) = self.temporal_sub
        {
            if ts.temporal_id_min > TEMPORAL_ID_MASK || ts.temporal_id_max > TEMPORAL_ID_MASK {
                return Err(Error::ValueOutOfRange {
                    field: "temporal_id",
                    reason: "exceeds 3-bit field",
                });
            }
            buf[HEADER_LEN + 13] = (broadcast_common::len::fit_mask(
                ts.temporal_id_min,
                TEMPORAL_ID_MASK,
                "temporal_id_min",
            )? << 5)
                | broadcast_common::len::fit_mask(
                    ts.reserved_min,
                    TEMPORAL_RESERVED_MASK,
                    "reserved_min",
                )?;
            buf[HEADER_LEN + 14] = (broadcast_common::len::fit_mask(
                ts.temporal_id_max,
                TEMPORAL_ID_MASK,
                "temporal_id_max",
            )? << 5)
                | broadcast_common::len::fit_mask(
                    ts.reserved_max,
                    TEMPORAL_RESERVED_MASK,
                    "reserved_max",
                )?;
        }

        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for HevcVideoDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "HEVC_VIDEO";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_no_temporal() {
        let orig = HevcVideoDescriptor {
            profile_space: 1,
            tier_flag: true,
            profile_idc: 2,
            profile_compatibility_indication: 0xDEADBEEF,
            progressive_source_flag: true,
            interlaced_source_flag: false,
            non_packed_constraint_flag: true,
            frame_only_constraint_flag: false,
            copied_44bits: 0x123456789AB & COPIED_44_MASK,
            level_idc: 0x99,
            temporal_layer_subset_flag: false,
            hevc_still_present_flag: true,
            hevc_24hr_picture_present_flag: false,
            sub_pic_hrd_params_not_present_flag: true,
            hdr_wcg_idc: HdrWcgIdc::HdrAndWcg,
            reserved: RESERVED_DEFAULT,
            temporal_sub: None,
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = HevcVideoDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn hdr_wcg_idc_is_low_two_bits_of_byte12() {
        // byte12 = tls(7)|still(6)|24hr(5)|sub_pic(4)|reserved(3:2)|HDR_WCG_idc(1:0).
        // Set the reserved bits [3:2] = '11' and HDR_WCG_idc [1:0] = '10' (=2, HdrAndWcg).
        // A parser that read the wrong bits ([3:2]) would decode 3 (NoIndication).
        let body12 = 0b0000_1110u8; // reserved=0b11, hdr_wcg_idc=0b10
        let buf = [
            TAG, 13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, body12,
        ];
        let d = HevcVideoDescriptor::parse(&buf).unwrap();
        assert_eq!(d.hdr_wcg_idc, HdrWcgIdc::HdrAndWcg);
        // Reserved bits must not leak into any field; Table 2-113 `reserved(2)`
        // is emitted '11' and parse ignores both bits.
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        // body byte 12 sits at buffer index HEADER_LEN + 12 = 14.
        assert_eq!(out[14] & 0x03, 0b10, "HDR_WCG_idc must occupy bits [1:0]");
        assert_eq!(
            out[14] & 0x0C,
            0x0C,
            "reserved bits [3:2] must serialize as 11"
        );
    }

    /// Real HEVC_video_descriptor bytes from the TSDuck test captures
    /// (`test-010.ts`, `test-083.ts`, ...): the flag byte is `0x9F` (reserved
    /// bits `[3:2]` = `11`) and the temporal sub-block is `1F 1F`
    /// (`reserved(5)` all ones). The serializer used to clear bit 3 and the
    /// sub-block reserved bits, so these did not re-serialize byte-identically.
    #[test]
    fn real_capture_bytes_round_trip_identically() {
        let real = [
            0x38, 0x0F, 0x02, 0x20, 0x00, 0x00, 0x00, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x99,
            0x9F, 0x1F, 0x1F,
        ];
        let d = HevcVideoDescriptor::parse(&real).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, real);
    }

    #[test]
    fn round_trip_with_temporal() {
        let orig = HevcVideoDescriptor {
            profile_space: 3,
            tier_flag: false,
            profile_idc: 0x0A,
            profile_compatibility_indication: 0xCAFEBABE,
            progressive_source_flag: false,
            interlaced_source_flag: true,
            non_packed_constraint_flag: false,
            frame_only_constraint_flag: true,
            copied_44bits: 0xABCDEF01234 & COPIED_44_MASK,
            level_idc: 0x5A,
            temporal_layer_subset_flag: true,
            hevc_still_present_flag: false,
            hevc_24hr_picture_present_flag: true,
            sub_pic_hrd_params_not_present_flag: false,
            hdr_wcg_idc: HdrWcgIdc::Sdr,
            reserved: RESERVED_DEFAULT,
            temporal_sub: Some(HevcTemporalSub::new(5, 7)),
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = HevcVideoDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn round_trip_nonzero_copied_44bits() {
        // deliberately set all 44 bits
        let orig = HevcVideoDescriptor {
            profile_space: 0,
            tier_flag: false,
            profile_idc: 1,
            profile_compatibility_indication: 0,
            progressive_source_flag: false,
            interlaced_source_flag: false,
            non_packed_constraint_flag: false,
            frame_only_constraint_flag: false,
            copied_44bits: COPIED_44_MASK, // all 44 bits set
            level_idc: 0x3C,
            temporal_layer_subset_flag: false,
            hevc_still_present_flag: false,
            hevc_24hr_picture_present_flag: false,
            sub_pic_hrd_params_not_present_flag: false,
            hdr_wcg_idc: HdrWcgIdc::NoIndication,
            reserved: RESERVED_DEFAULT,
            temporal_sub: None,
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = HevcVideoDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
        assert_eq!(reparsed.copied_44bits, COPIED_44_MASK);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let buf = [
            0x02, 13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let err = HevcVideoDescriptor::parse(&buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        // present-but-short body (2 bytes < FIXED_BODY_LEN) → descriptor's own check.
        let err = HevcVideoDescriptor::parse(&[TAG, 2, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_wide_raw_sub_byte_fields() {
        // profile_space(2), profile_idc(5), temporal_id(3): pre-fix silently
        // clipped via mask (r03-W3).
        let base = HevcVideoDescriptor {
            profile_space: 1,
            tier_flag: true,
            profile_idc: 2,
            profile_compatibility_indication: 0xDEADBEEF,
            progressive_source_flag: true,
            interlaced_source_flag: false,
            non_packed_constraint_flag: true,
            frame_only_constraint_flag: false,
            copied_44bits: 0x123456789AB & COPIED_44_MASK,
            level_idc: 0x99,
            temporal_layer_subset_flag: false,
            hevc_still_present_flag: false,
            hevc_24hr_picture_present_flag: false,
            sub_pic_hrd_params_not_present_flag: false,
            hdr_wcg_idc: HdrWcgIdc::Sdr,
            reserved: RESERVED_DEFAULT,
            temporal_sub: None,
        };
        let mut d = base.clone();
        d.profile_space = 4;
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::ValueOutOfRange {
                field: "profile_space",
                ..
            }
        ));
        let mut d = base.clone();
        d.profile_idc = 0x20;
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::ValueOutOfRange {
                field: "profile_idc",
                ..
            }
        ));
        let mut d = base.clone();
        d.temporal_layer_subset_flag = true;
        d.temporal_sub = Some(HevcTemporalSub::new(8, 0));
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::ValueOutOfRange { .. }
        ));
    }
    #[test]
    fn serialize_rejects_flag_option_presence_mismatch() {
        // r03-W6: serialized_len reserves the temporal-sub bytes from the
        // flag; with the Option empty the old writer left 2 stale bytes yet
        // returned Ok.
        let mut d = HevcVideoDescriptor {
            profile_space: 1,
            tier_flag: true,
            profile_idc: 2,
            profile_compatibility_indication: 0xDEADBEEF,
            progressive_source_flag: true,
            interlaced_source_flag: false,
            non_packed_constraint_flag: true,
            frame_only_constraint_flag: false,
            copied_44bits: 0x123456789AB & COPIED_44_MASK,
            level_idc: 0x99,
            temporal_layer_subset_flag: true,
            hevc_still_present_flag: false,
            hevc_24hr_picture_present_flag: false,
            sub_pic_hrd_params_not_present_flag: false,
            hdr_wcg_idc: HdrWcgIdc::Sdr,
            reserved: RESERVED_DEFAULT,
            temporal_sub: None,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::ValueOutOfRange {
                field: "temporal_sub",
                ..
            }
        ));
        d.temporal_layer_subset_flag = false;
        d.temporal_sub = Some(HevcTemporalSub::new(0, 1));
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut buf).is_err());
    }

    #[test]
    fn parse_rejects_body_without_mandatory_flags_byte() {
        // Table 2-113 makes byte 12 mandatory; a 12-byte body used to be
        // accepted with the flags defaulted and then re-serialized as 13
        // bytes (r03-W12).
        let mut bytes = vec![TAG, 12];
        bytes.extend(std::iter::repeat_n(0u8, 12));
        assert!(matches!(
            HevcVideoDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_temporal_sub_block_when_flag_clear() {
        // 14 bytes with temporal_layer_subset_flag = 0: two bytes past the
        // mandatory flags byte with nothing to carry them.
        let mut bytes = vec![TAG, 14];
        bytes.extend(std::iter::repeat_n(0u8, 13));
        bytes.push(0x00);
        assert!(matches!(
            HevcVideoDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    /// H.222.0 Table 2-113. Every reserved region (byte-12 `reserved(2)` and
    /// both temporal-sub `reserved(5)`) is preserved verbatim, incl. the
    /// zero-bit-3 form the pre-fix serializer used to emit.
    #[test]
    fn hevc_video_reserved_preserved() {
        for r in 0u8..=3 {
            for rt in [0u8, 1, 0x0A, 0x15, 0x1F] {
                let bytes = [
                    TAG,
                    15,
                    0x02,
                    0x20,
                    0x00,
                    0x00,
                    0x00,
                    0xB0,
                    0x00,
                    0x00,
                    0x00,
                    0x00,
                    0x00,
                    0x99,
                    0x82 | (r << 2) | 0x90,
                    rt,
                    0x60 | rt,
                ];
                let d = HevcVideoDescriptor::parse(&bytes).unwrap();
                assert_eq!(d.reserved, r);
                let ts = d.temporal_sub.as_ref().unwrap();
                assert_eq!((ts.reserved_min, ts.reserved_max), (rt, rt));
                let mut out = vec![0u8; d.serialized_len()];
                d.serialize_into(&mut out).unwrap();
                assert_eq!(out, bytes);
            }
        }
    }

    #[test]
    fn hevc_video_default_is_spec_value() {
        let mut d = HevcVideoDescriptor::new(0, false, 2, 0x2000_0000, 0x99);
        d.temporal_layer_subset_flag = true;
        d.temporal_sub = Some(HevcTemporalSub::new(0, 3));
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out[14] & 0xFC,
            0x80 | 0x0C,
            "reserved(2) default is 11 at [3:2]"
        );
        assert_eq!(out[15], 0x1F);
        assert_eq!(out[16], 0x60 | 0x1F);
    }
}
