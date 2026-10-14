//! Video Stream Descriptor — ISO/IEC 13818-1 §2.6.2 (tag 0x02).
//!
//! Describes the elementary stream as a video stream, including frame rate
//! and profile/level constraints.

use super::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for video_stream_descriptor.
pub const TAG: u8 = 0x02;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 3;
const BODY_MPEG1_LEN: u8 = 1;
/// Default of the trailing 5-bit `reserved` of the MPEG-2 branch (H.222.0
/// Table 2-46): clause 2.1 "reserved" = `1` (all 9 editions 07/1995-08/2023,
/// audit §5; confirmed by the TSDuck oracle PMT and the real
/// `shared/real-pmt.bin` capture).
pub const RESERVED_5_DEFAULT: u8 = 0b1_1111;
const RESERVED_5_MASK: u8 = 0x1F;

/// `frame_rate_code(4)` field mask inside byte 0: bits `[6:3]`; masks the
/// `Reserved(v)` payload so it cannot bleed into `multiple_frame_rate_flag`
/// when shifted (r03-W2).
const FRAME_RATE_CODE_MASK: u8 = 0x0F;

/// Frame rate code — ISO/IEC 13818-1 Table 2-47.
///
/// 4-bit code in the video_stream_descriptor. The "also includes" column
/// (`multiple_frame_rate_flag = 1`) is handled by the descriptor-level
/// `multiple_frame_rate_flag` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FrameRateCode {
    /// 0x0 — Forbidden.
    Forbidden,
    /// 0x1 — 23.976 Hz.
    Frame23_976,
    /// 0x2 — 24.0 Hz (includes 23.976).
    Frame24_0,
    /// 0x3 — 25.0 Hz.
    Frame25_0,
    /// 0x4 — 29.97 Hz (includes 23.976).
    Frame29_97,
    /// 0x5 — 30.0 Hz (includes 23.976, 24.0, 29.97).
    Frame30_0,
    /// 0x6 — 50.0 Hz (includes 25.0).
    Frame50_0,
    /// 0x7 — 59.94 Hz (includes 23.976, 29.97).
    Frame59_94,
    /// 0x8 — 60.0 Hz (includes 23.976, 24.0, 29.97, 30.0, 59.94).
    Frame60_0,
    /// 0x9–0xF — Reserved / unrecognised value, preserved verbatim
    /// for byte-identical round-trip.
    Reserved(u8),
}

impl FrameRateCode {
    /// Construct from a raw byte; unknown values are preserved as `Reserved`
    /// for byte-identical round-trip.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x0 => Self::Forbidden,
            0x1 => Self::Frame23_976,
            0x2 => Self::Frame24_0,
            0x3 => Self::Frame25_0,
            0x4 => Self::Frame29_97,
            0x5 => Self::Frame30_0,
            0x6 => Self::Frame50_0,
            0x7 => Self::Frame59_94,
            0x8 => Self::Frame60_0,
            v => Self::Reserved(v),
        }
    }

    /// Return the raw byte value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Forbidden => 0x0,
            Self::Frame23_976 => 0x1,
            Self::Frame24_0 => 0x2,
            Self::Frame25_0 => 0x3,
            Self::Frame29_97 => 0x4,
            Self::Frame30_0 => 0x5,
            Self::Frame50_0 => 0x6,
            Self::Frame59_94 => 0x7,
            Self::Frame60_0 => 0x8,
            Self::Reserved(v) => v,
        }
    }

    /// Returns a human-readable spec name for this value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::Frame23_976 => "23.976",
            Self::Frame24_0 => "24.0",
            Self::Frame25_0 => "25.0",
            Self::Frame29_97 => "29.97",
            Self::Frame30_0 => "30.0",
            Self::Frame50_0 => "50.0",
            Self::Frame59_94 => "59.94",
            Self::Frame60_0 => "60.0",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(FrameRateCode, Reserved);

/// Video Stream Descriptor.
///
/// If `mpeg_1_only_flag` is true, `profile_and_level_indication`,
/// `chroma_format`, and `frame_rate_extension_flag` are absent (body
/// is only 1 byte instead of 3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct VideoStreamDescriptor {
    /// 1 — more than one frame rate may be present (see Table 2-47 "also includes").
    pub multiple_frame_rate_flag: bool,
    /// Frame rate code (Table 2-47).
    pub frame_rate_code: FrameRateCode,
    /// 1 — stream constrained to MPEG-1 (no profile/level/chroma fields follow).
    pub mpeg_1_only_flag: bool,
    /// Constrained parameters flag.
    pub constrained_parameter_flag: bool,
    /// Still picture flag.
    pub still_picture_flag: bool,
    /// Profile and level indication — only when `mpeg_1_only_flag` is false.
    pub profile_and_level_indication: Option<u8>,
    /// Chroma format (H.262 §6.3.11) — only when `mpeg_1_only_flag` is false.
    pub chroma_format: Option<u8>,
    /// Frame rate extension flag — only when `mpeg_1_only_flag` is false.
    pub frame_rate_extension_flag: Option<bool>,
    /// Trailing `reserved(5)` of the MPEG-2 branch (5 bits, right-aligned);
    /// not on the wire when `mpeg_1_only_flag` is set. Captured on parse,
    /// re-emitted verbatim; the constructors use [`RESERVED_5_DEFAULT`].
    pub reserved_5: u8,
    /// Body bytes after the defined fields (after byte 0 in the MPEG-1 branch,
    /// after byte 2 otherwise): tolerated on parse and re-emitted verbatim.
    pub trailing: Vec<u8>,
}

impl VideoStreamDescriptor {
    /// Build an MPEG-1-only descriptor (`mpeg_1_only_flag` = 1, 1-byte body).
    #[must_use]
    pub const fn mpeg1(
        multiple_frame_rate_flag: bool,
        frame_rate_code: FrameRateCode,
        constrained_parameter_flag: bool,
        still_picture_flag: bool,
    ) -> Self {
        Self {
            multiple_frame_rate_flag,
            frame_rate_code,
            mpeg_1_only_flag: true,
            constrained_parameter_flag,
            still_picture_flag,
            profile_and_level_indication: None,
            chroma_format: None,
            frame_rate_extension_flag: None,
            reserved_5: RESERVED_5_DEFAULT,
            trailing: Vec::new(),
        }
    }

    /// Build an MPEG-2 descriptor (`mpeg_1_only_flag` = 0, 3-byte body) with
    /// the spec-default reserved bits.
    #[must_use]
    pub const fn mpeg2(
        multiple_frame_rate_flag: bool,
        frame_rate_code: FrameRateCode,
        constrained_parameter_flag: bool,
        still_picture_flag: bool,
        profile_and_level_indication: u8,
        chroma_format: u8,
        frame_rate_extension_flag: bool,
    ) -> Self {
        Self {
            multiple_frame_rate_flag,
            frame_rate_code,
            mpeg_1_only_flag: false,
            constrained_parameter_flag,
            still_picture_flag,
            profile_and_level_indication: Some(profile_and_level_indication),
            chroma_format: Some(chroma_format),
            frame_rate_extension_flag: Some(frame_rate_extension_flag),
            reserved_5: RESERVED_5_DEFAULT,
            trailing: Vec::new(),
        }
    }
}

impl<'a> Parse<'a> for VideoStreamDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "VideoStreamDescriptor",
            "unexpected tag for video_stream_descriptor",
        )?;
        if body.is_empty() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "video_stream_descriptor length must be at least 1",
            });
        }
        let b0 = body[0];
        let multiple_frame_rate_flag = (b0 & 0x80) != 0;
        let frame_rate_code = FrameRateCode::from_u8((b0 >> 3) & 0x0F);
        let mpeg_1_only_flag = (b0 & 0x04) != 0;
        let constrained_parameter_flag = (b0 & 0x02) != 0;
        let still_picture_flag = (b0 & 0x01) != 0;

        if mpeg_1_only_flag {
            Ok(Self {
                multiple_frame_rate_flag,
                frame_rate_code,
                mpeg_1_only_flag,
                constrained_parameter_flag,
                still_picture_flag,
                profile_and_level_indication: None,
                chroma_format: None,
                frame_rate_extension_flag: None,
                reserved_5: RESERVED_5_DEFAULT,
                trailing: body[BODY_MPEG1_LEN as usize..].to_vec(),
            })
        } else {
            if body.len() < (BODY_LEN as usize) {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "video_stream_descriptor too short for MPEG-2 fields",
                });
            }
            let b1 = body[1];
            let b2 = body[2];
            let profile_and_level_indication = b1;
            let chroma_format = (b2 >> 6) & 0x03;
            let frame_rate_extension_flag = (b2 & 0x20) != 0;
            Ok(Self {
                multiple_frame_rate_flag,
                frame_rate_code,
                mpeg_1_only_flag,
                constrained_parameter_flag,
                still_picture_flag,
                profile_and_level_indication: Some(profile_and_level_indication),
                chroma_format: Some(chroma_format),
                frame_rate_extension_flag: Some(frame_rate_extension_flag),
                reserved_5: b2 & RESERVED_5_MASK,
                trailing: body[BODY_LEN as usize..].to_vec(),
            })
        }
    }
}

impl Serialize for VideoStreamDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        if self.mpeg_1_only_flag {
            HEADER_LEN + (BODY_MPEG1_LEN as usize) + self.trailing.len()
        } else {
            HEADER_LEN + (BODY_LEN as usize) + self.trailing.len()
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let b0 = ((self.multiple_frame_rate_flag as u8) << 7)
            | (broadcast_common::len::fit_mask(
                self.frame_rate_code.to_u8(),
                FRAME_RATE_CODE_MASK,
                "frame_rate_code",
            )? << 3)
            | ((self.mpeg_1_only_flag as u8) << 2)
            | ((self.constrained_parameter_flag as u8) << 1)
            | (self.still_picture_flag as u8);
        buf[HEADER_LEN] = b0;
        if !self.mpeg_1_only_flag {
            // All three fields are on the wire together whenever
            // `mpeg_1_only_flag` is clear; fabricating 0/false would misframe
            // (r03-W5).
            let pal = self
                .profile_and_level_indication
                .ok_or(Error::ValueOutOfRange {
                    field: "profile_and_level_indication",
                    reason: "required when mpeg_1_only_flag is false",
                })?;
            let chroma = broadcast_common::len::fit_mask(
                self.chroma_format.ok_or(Error::ValueOutOfRange {
                    field: "chroma_format",
                    reason: "required when mpeg_1_only_flag is false",
                })?,
                0x03,
                "chroma_format",
            )?;
            let fre = self
                .frame_rate_extension_flag
                .ok_or(Error::ValueOutOfRange {
                    field: "frame_rate_extension_flag",
                    reason: "required when mpeg_1_only_flag is false",
                })? as u8;
            buf[HEADER_LEN + 1] = pal;
            buf[HEADER_LEN + 2] = (chroma << 6)
                | (fre << 5)
                | broadcast_common::len::fit_mask(self.reserved_5, RESERVED_5_MASK, "reserved_5")?;
        }
        let fixed = if self.mpeg_1_only_flag {
            BODY_MPEG1_LEN as usize
        } else {
            BODY_LEN as usize
        };
        buf[HEADER_LEN + fixed..len].copy_from_slice(&self.trailing);
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for VideoStreamDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "VIDEO_STREAM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mpeg_2() {
        let bytes = [
            TAG,
            3,           // tag + length
            0b1010_0011, // multiple=1, frame_rate=4 (29.97), mpeg1=0, constrained=1, still=1
            0xDE,        // profile_and_level_indication
            0b1110_0000, // chroma_format=3, frame_rate_extension_flag=1, reserved=0
        ];
        let d = VideoStreamDescriptor::parse(&bytes).unwrap();
        assert!(d.multiple_frame_rate_flag);
        assert_eq!(d.frame_rate_code, FrameRateCode::Frame29_97);
        assert!(!d.mpeg_1_only_flag);
        assert!(d.constrained_parameter_flag);
        assert!(d.still_picture_flag);
        assert_eq!(d.profile_and_level_indication, Some(0xDE));
        assert_eq!(d.chroma_format, Some(3));
        assert_eq!(d.frame_rate_extension_flag, Some(true));
    }

    #[test]
    fn parse_mpeg_1() {
        let bytes = [
            TAG,
            1,
            0b0001_1101, // multiple=0, frame_rate=3 (25.0), mpeg1=1, constrained=0, still=1
        ];
        let d = VideoStreamDescriptor::parse(&bytes).unwrap();
        assert!(!d.multiple_frame_rate_flag);
        assert_eq!(d.frame_rate_code, FrameRateCode::Frame25_0);
        assert!(d.mpeg_1_only_flag);
        assert!(!d.constrained_parameter_flag);
        assert!(d.still_picture_flag);
        assert!(d.profile_and_level_indication.is_none());
        assert!(d.chroma_format.is_none());
        assert!(d.frame_rate_extension_flag.is_none());
    }

    #[test]
    fn serialize_round_trip_mpeg_2() {
        let d = VideoStreamDescriptor::mpeg2(
            true,
            FrameRateCode::Frame60_0,
            false,
            true,
            0xAB,
            1,
            false,
        );
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = VideoStreamDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_round_trip_mpeg_1() {
        let d = VideoStreamDescriptor::mpeg1(false, FrameRateCode::Frame23_976, true, false);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = VideoStreamDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = VideoStreamDescriptor::parse(&[0x03, 1, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x03, .. }));
    }

    #[test]
    fn parse_rejects_empty_body() {
        let err = VideoStreamDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn frame_rate_code_round_trip() {
        for v in 0u8..=0x0F {
            assert_eq!(
                FrameRateCode::from_u8(v).to_u8(),
                v,
                "round-trip failed for {v:#04x}"
            );
        }
    }

    #[test]
    fn frame_rate_code_name() {
        assert_eq!(FrameRateCode::Frame23_976.name(), "23.976");
        assert_eq!(FrameRateCode::Frame25_0.name(), "25.0");
        assert_eq!(FrameRateCode::Reserved(0xA).name(), "reserved");
        assert_eq!(FrameRateCode::Forbidden.name(), "forbidden");
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = VideoStreamDescriptor::mpeg1(false, FrameRateCode::Frame25_0, false, false);
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
    #[test]
    fn serialize_rejects_missingmpeg2_conditional_fields() {
        // r03-W5: with `mpeg_1_only_flag` false the three conditional fields
        // are mandatory; serialize used to fabricate 0 / false via
        // `unwrap_or` instead of erroring.
        let mut d = VideoStreamDescriptor::mpeg2(
            false,
            FrameRateCode::Frame25_0,
            false,
            false,
            0,
            1,
            false,
        );
        d.profile_and_level_indication = None;
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::ValueOutOfRange {
                field: "profile_and_level_indication",
                ..
            }
        ));
    }

    /// H.222.0 Table 2-46. Parsed reserved bits (and any trailing body bytes)
    /// are preserved; the old normalize-to-ones behaviour is gone.
    #[test]
    fn video_stream_reserved_preserved() {
        for r in 0u8..=0x1F {
            let bytes = [TAG, 3, 0b1010_0011, 0xDE, 0xE0 | r];
            let d = VideoStreamDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved_5, r);
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
        // Tolerated trailing bytes survive in both branches.
        for bytes in [
            &[TAG, 3 + 2, 0b1010_0011, 0xDE, 0xE5, 0xAA, 0xBB][..],
            &[TAG, 1 + 2, 0b0001_1101, 0xAA, 0xBB][..],
        ] {
            let d = VideoStreamDescriptor::parse(bytes).unwrap();
            assert_eq!(d.trailing, [0xAA, 0xBB]);
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn video_stream_default_is_spec_value() {
        let d = VideoStreamDescriptor::mpeg2(
            true,
            FrameRateCode::Frame29_97,
            true,
            true,
            0xDE,
            3,
            true,
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 3, 0b1010_0011, 0xDE, 0xFF]);
    }
}
