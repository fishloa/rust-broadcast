//! Video Stream Descriptor — ISO/IEC 13818-1 §2.6.2 (tag 0x02).
//!
//! Describes the elementary stream as a video stream, including frame rate
//! and profile/level constraints.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for video_stream_descriptor.
pub const TAG: u8 = 0x02;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 3;
const BODY_MPEG1_LEN: u8 = 1;

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
    pub fn to_u8(self) -> u8 {
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
dvb_common::impl_spec_display!(FrameRateCode, Reserved);

/// Video Stream Descriptor.
///
/// If `mpeg_1_only_flag` is true, `profile_and_level_indication`,
/// `chroma_format`, and `frame_rate_extension_flag` are absent (body
/// is only 1 byte instead of 3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
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
            })
        }
    }
}

impl Serialize for VideoStreamDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        if self.mpeg_1_only_flag {
            HEADER_LEN + (BODY_MPEG1_LEN as usize)
        } else {
            HEADER_LEN + (BODY_LEN as usize)
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
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        let b0 = ((self.multiple_frame_rate_flag as u8) << 7)
            | (self.frame_rate_code.to_u8() << 3)
            | ((self.mpeg_1_only_flag as u8) << 2)
            | ((self.constrained_parameter_flag as u8) << 1)
            | (self.still_picture_flag as u8);
        buf[HEADER_LEN] = b0;
        if !self.mpeg_1_only_flag {
            buf[HEADER_LEN + 1] = self.profile_and_level_indication.unwrap_or(0);
            let chroma = self.chroma_format.unwrap_or(0) & 0x03;
            let fre = self.frame_rate_extension_flag.unwrap_or(false) as u8;
            buf[HEADER_LEN + 2] = (chroma << 6) | (fre << 5);
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for VideoStreamDescriptor {
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
        let d = VideoStreamDescriptor {
            multiple_frame_rate_flag: true,
            frame_rate_code: FrameRateCode::Frame60_0,
            mpeg_1_only_flag: false,
            constrained_parameter_flag: false,
            still_picture_flag: true,
            profile_and_level_indication: Some(0xAB),
            chroma_format: Some(1),
            frame_rate_extension_flag: Some(false),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = VideoStreamDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_round_trip_mpeg_1() {
        let d = VideoStreamDescriptor {
            multiple_frame_rate_flag: false,
            frame_rate_code: FrameRateCode::Frame23_976,
            mpeg_1_only_flag: true,
            constrained_parameter_flag: true,
            still_picture_flag: false,
            profile_and_level_indication: None,
            chroma_format: None,
            frame_rate_extension_flag: None,
        };
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
        let d = VideoStreamDescriptor {
            multiple_frame_rate_flag: false,
            frame_rate_code: FrameRateCode::Frame25_0,
            mpeg_1_only_flag: true,
            constrained_parameter_flag: false,
            still_picture_flag: false,
            profile_and_level_indication: None,
            chroma_format: None,
            frame_rate_extension_flag: None,
        };
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
