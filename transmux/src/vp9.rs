//! VP9 in ISOBMFF — `vp09` VisualSampleEntry + `vpcC` config box.
//!
//! WebM Project "VP Codec ISO Media File Format Binding"
//! (<https://www.webmproject.org/vp9/mp4/>, VPCodecConfigurationBox).
//!
//! # Types
//!
//! | Box | FourCC | Description |
//! |-----|--------|-------------|
//! | [`Vp9ConfigurationBox`] | `vpcC` | `FullBox` v1 VPCodecConfigurationRecord |
//! | [`Vp9SampleEntry`] | `vp09` | VisualSampleEntry carrying `vpcC` |

use crate::error::{Error, Result};
use crate::sample_entries::VisualSampleEntryFields;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// FourCC of the VP9 config box.
pub const VPCC_FOURCC: [u8; 4] = *b"vpcC";
/// FourCC of the VP9 sample entry.
pub const VP09_FOURCC: [u8; 4] = *b"vp09";
/// 8-byte box header.
const BOX_HDR: usize = 8;
/// FullBox extension: version(1) + flags(3).
const FULL_HDR: usize = 4;
/// Fixed length of the VPCodecConfigurationRecord (before `codecInitializationData`).
const VPCC_RECORD_FIXED: usize = 8;

/// VPCodecConfigurationBox (`vpcC`) — a `FullBox(version=1, 0)` VP9 config record.
///
/// Record layout after the FullBox header:
/// `profile(8)` | `level(8)` | `bit_depth[7:4]` | `chroma_subsampling[3:1]` |
/// `video_full_range_flag[0:0]` | `colour_primaries(8)` |
/// `transfer_characteristics(8)` | `matrix_coefficients(8)` |
/// `codec_initialization_data_size(16)` | `codec_initialization_data[]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Vp9ConfigurationBox {
    /// FullBox version (1).
    pub version: u8,
    /// FullBox flags (0).
    pub flags: u32,
    /// VP9 profile (0-3).
    pub profile: u8,
    /// VP9 level.
    pub level: u8,
    /// Bit depth in bits (8, 10, or 12).
    pub bit_depth: u8,
    /// `chroma_subsampling` (3 bits).
    pub chroma_subsampling: u8,
    /// `video_full_range_flag`.
    pub video_full_range_flag: bool,
    /// `colour_primaries` (CICP).
    pub colour_primaries: u8,
    /// `transfer_characteristics` (CICP).
    pub transfer_characteristics: u8,
    /// `matrix_coefficients` (CICP).
    pub matrix_coefficients: u8,
    /// `codec_initialization_data` (MUST be empty for VP8/VP9).
    pub codec_initialization_data: Vec<u8>,
}

impl Vp9ConfigurationBox {
    /// RFC 6381 codec string `vp09.PP.LL.DD` (profile, level, bit depth).
    pub fn rfc6381(&self) -> alloc::string::String {
        use alloc::format;
        format!(
            "vp09.{:02}.{:02}.{:02}",
            self.profile, self.level, self.bit_depth
        )
    }
}

impl<'a> Parse<'a> for Vp9ConfigurationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FULL_HDR + VPCC_RECORD_FIXED {
            return Err(Error::BufferTooShort {
                need: FULL_HDR + VPCC_RECORD_FIXED,
                have: bytes.len(),
                what: "vpcC body",
            });
        }
        let version = bytes[0];
        let flags = u32::from_be_bytes([0, bytes[1], bytes[2], bytes[3]]);
        let r = &bytes[FULL_HDR..];
        let profile = r[0];
        let level = r[1];
        let bit_depth = r[2] >> 4;
        let chroma_subsampling = (r[2] >> 1) & 0x07;
        let video_full_range_flag = (r[2] & 0x01) != 0;
        let colour_primaries = r[3];
        let transfer_characteristics = r[4];
        let matrix_coefficients = r[5];
        let init_size = u16::from_be_bytes([r[6], r[7]]) as usize;
        let init_start = FULL_HDR + VPCC_RECORD_FIXED;
        let init_end = (init_start + init_size).min(bytes.len());
        let codec_initialization_data = bytes[init_start..init_end].to_vec();
        Ok(Self {
            version,
            flags,
            profile,
            level,
            bit_depth,
            chroma_subsampling,
            video_full_range_flag,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            codec_initialization_data,
        })
    }
}

impl Serialize for Vp9ConfigurationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        FULL_HDR + VPCC_RECORD_FIXED + self.codec_initialization_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[1..4].copy_from_slice(&fb[1..]);
        let r = &mut buf[FULL_HDR..];
        r[0] = self.profile;
        r[1] = self.level;
        r[2] = (self.bit_depth << 4)
            | ((self.chroma_subsampling & 0x07) << 1)
            | (self.video_full_range_flag as u8);
        r[3] = self.colour_primaries;
        r[4] = self.transfer_characteristics;
        r[5] = self.matrix_coefficients;
        let size = self.codec_initialization_data.len() as u16;
        r[6..8].copy_from_slice(&size.to_be_bytes());
        r[VPCC_RECORD_FIXED..VPCC_RECORD_FIXED + self.codec_initialization_data.len()]
            .copy_from_slice(&self.codec_initialization_data);
        Ok(need)
    }
}

/// VP9 sample entry (`vp09`) — a `VisualSampleEntry` carrying a `vpcC` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Vp9SampleEntry {
    /// Fixed VisualSampleEntry fields.
    pub visual: VisualSampleEntryFields,
    /// The `vpcC` configuration box.
    pub config: Vp9ConfigurationBox,
}

impl Vp9SampleEntry {
    /// Parse from full box bytes (including the 8-byte header).
    pub fn parse_entry(bytes: &[u8]) -> Result<Self> {
        let visual = VisualSampleEntryFields::parse_body(bytes, "vp09")?;
        let region = &bytes[BOX_HDR + VisualSampleEntryFields::serialized_len()..];
        let vpcc = crate::sample_entries::find_config_box(region, &VPCC_FOURCC).ok_or(
            Error::BufferTooShort {
                need: 0,
                have: 0,
                what: "vp09 missing vpcC",
            },
        )?;
        let config = Vp9ConfigurationBox::parse(&vpcc[BOX_HDR..])?;
        Ok(Self { visual, config })
    }
}

impl Serialize for Vp9SampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + VisualSampleEntryFields::serialized_len() + BOX_HDR + self.config.serialized_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&VP09_FOURCC);
        c += 4;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        let vpcc_len = BOX_HDR + self.config.serialized_len();
        buf[c..c + 4].copy_from_slice(&(vpcc_len as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&VPCC_FOURCC);
        c += 4;
        c += self.config.serialize_into(&mut buf[c..])?;
        Ok(c)
    }
}
