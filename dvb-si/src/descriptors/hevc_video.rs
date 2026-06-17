//! HEVC Video Descriptor — ISO/IEC 13818-1 §2.6.95 (tag 0x38).
//!
//! Describes the profile, tier, level, and constraint flags for an
//! HEVC/H.265 video elementary stream. The `copied_44bits` field is a
//! 44-bit non-byte-aligned region stored as a u64 masked to 44 bits.
//! The temporal_id block is conditional on `temporal_layer_subset_flag`.

use super::descriptor_body;
use super::hdr_wcg_idc::HdrWcgIdc;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for HEVC_video_descriptor.
pub const TAG: u8 = 0x38;
const HEADER_LEN: usize = 2;
const COPIED_44_MASK: u64 = (1 << 44) - 1;
/// Fixed body length without temporal_id sub-block.
const FIXED_BODY_LEN: u8 = 12;
/// Temporal sub-block length when present.
const TEMPORAL_SUB_LEN: u8 = 2;

/// Temporal layer subset sub-block — present when `temporal_layer_subset_flag` is true.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HevcTemporalSub {
    /// Minimum temporal_id value.
    pub temporal_id_min: u8,
    /// Maximum temporal_id value.
    pub temporal_id_max: u8,
}

/// HEVC Video Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
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
    /// Temporal layer sub-block, when present.
    pub temporal_sub: Option<HevcTemporalSub>,
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

        let temporal_layer_subset_flag;
        let hevc_still_present_flag;
        let hevc_24hr_picture_present_flag;
        let sub_pic_hrd_params_not_present_flag;
        let hdr_wcg_idc;
        let temporal_sub;

        if body.len() > (FIXED_BODY_LEN as usize) {
            // byte 12: tls(7)|still(6)|24hr(5)|sub_pic(4)|reserved(3:2)|HDR_WCG_idc(1:0)
            let b12 = body[12];
            temporal_layer_subset_flag = (b12 & 0x80) != 0;
            hevc_still_present_flag = (b12 & 0x40) != 0;
            hevc_24hr_picture_present_flag = (b12 & 0x20) != 0;
            sub_pic_hrd_params_not_present_flag = (b12 & 0x10) != 0;
            hdr_wcg_idc = HdrWcgIdc::from_u8(b12 & 0x03);

            if temporal_layer_subset_flag {
                if body.len() < (FIXED_BODY_LEN + TEMPORAL_SUB_LEN) as usize {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "HEVC_video_descriptor too short for temporal sub-block",
                    });
                }
                let b13 = body[13];
                let b14 = body[14];
                temporal_sub = Some(HevcTemporalSub {
                    temporal_id_min: b13 >> 5,
                    temporal_id_max: b14 >> 5,
                });
            } else {
                temporal_sub = None;
            }
        } else {
            // No extra bytes beyond fixed — default flags
            temporal_layer_subset_flag = false;
            hevc_still_present_flag = false;
            hevc_24hr_picture_present_flag = false;
            sub_pic_hrd_params_not_present_flag = false;
            hdr_wcg_idc = HdrWcgIdc::NoIndication;
            temporal_sub = None;
        }

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
        buf[HEADER_LEN] =
            (self.profile_space << 6) | ((self.tier_flag as u8) << 5) | (self.profile_idc & 0x1F);

        // bytes 1..5: profile_compatibility_indication (32 bits big-endian)
        buf[HEADER_LEN + 1..HEADER_LEN + 5]
            .copy_from_slice(&self.profile_compatibility_indication.to_be_bytes());

        // byte 5: flags(4) | copied_44bits hi 4 bits
        let copied = self.copied_44bits & COPIED_44_MASK;
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
            | (self.hdr_wcg_idc.to_u8() & 0x03);

        if self.temporal_layer_subset_flag {
            if let Some(ref ts) = self.temporal_sub {
                buf[HEADER_LEN + 13] = ts.temporal_id_min << 5;
                buf[HEADER_LEN + 14] = ts.temporal_id_max << 5;
            }
        }

        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for HevcVideoDescriptor {
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
        // Reserved bits must not leak into any field and must serialize back as zero.
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        // body byte 12 sits at buffer index HEADER_LEN + 12 = 14.
        assert_eq!(out[14] & 0x03, 0b10, "HDR_WCG_idc must occupy bits [1:0]");
        assert_eq!(
            out[14] & 0x0C,
            0,
            "reserved bits [3:2] must serialize as zero"
        );
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
            temporal_sub: Some(HevcTemporalSub {
                temporal_id_min: 5,
                temporal_id_max: 7,
            }),
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
}
