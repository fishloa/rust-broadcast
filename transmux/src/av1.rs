//! AV1 in ISOBMFF — `av01` VisualSampleEntry + `av1C` config box.
//!
//! AOMedia "AV1 Codec ISO Media File Format Binding"
//! (<https://aomediacodec.github.io/av1-isobmff/>, §2.3 AV1CodecConfigurationBox).
//!
//! # Types
//!
//! | Box | FourCC | Description |
//! |-----|--------|-------------|
//! | [`Av1ConfigurationBox`] | `av1C` | AV1CodecConfigurationRecord |
//! | [`Av1SampleEntry`] | `av01` | VisualSampleEntry carrying `av1C` |
//!
//! The record's `configOBUs` tail carries an optional Sequence Header OBU verbatim;
//! it is preserved opaquely so the box round-trips byte-exact.

use crate::error::{Error, Result};
use crate::sample_entries::VisualSampleEntryFields;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// FourCC of the AV1 config box.
pub const AV1C_FOURCC: [u8; 4] = *b"av1C";
/// FourCC of the AV1 sample entry.
pub const AV01_FOURCC: [u8; 4] = *b"av01";
/// Fixed size of the `av1C` record before the trailing `configOBUs`.
const AV1C_FIXED_LEN: usize = 4;
/// 8-byte box header.
const BOX_HDR: usize = 8;
/// `marker(1)=1` in the high bit of byte 0.
const MARKER_BIT: u8 = 0x80;

/// AV1CodecConfigurationRecord (`av1C` box body) — AV1-ISOBMFF §2.3.
///
/// Layout of the 4-byte fixed prefix:
/// byte 0: `marker[7:7]=1` | `version[6:0]`;
/// byte 1: `seq_profile[7:5]` | `seq_level_idx_0[4:0]`;
/// byte 2: `seq_tier_0[7:7]` | `high_bitdepth[6:6]` | `twelve_bit[5:5]` |
/// `monochrome[4:4]` | `chroma_subsampling_x[3:3]` | `chroma_subsampling_y[2:2]` |
/// `chroma_sample_position[1:0]`;
/// byte 3: `reserved[7:5]=0` | `initial_presentation_delay_present[4:4]` |
/// (`initial_presentation_delay_minus_one` or `reserved`)`[3:0]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Av1ConfigurationBox {
    /// Record version (`1`).
    pub version: u8,
    /// `seq_profile` (3 bits) from the Sequence Header OBU.
    pub seq_profile: u8,
    /// `seq_level_idx[0]` (5 bits).
    pub seq_level_idx_0: u8,
    /// `seq_tier[0]` (1 bit).
    pub seq_tier_0: bool,
    /// `high_bitdepth` (1 bit).
    pub high_bitdepth: bool,
    /// `twelve_bit` (1 bit).
    pub twelve_bit: bool,
    /// `mono_chrome` (1 bit).
    pub monochrome: bool,
    /// `chroma_subsampling_x` (1 bit).
    pub chroma_subsampling_x: bool,
    /// `chroma_subsampling_y` (1 bit).
    pub chroma_subsampling_y: bool,
    /// `chroma_sample_position` (2 bits).
    pub chroma_sample_position: u8,
    /// `initial_presentation_delay_minus_one` (4 bits), present iff the
    /// `initial_presentation_delay_present` flag is set.
    pub initial_presentation_delay_minus_one: Option<u8>,
    /// Trailing `configOBUs` (≤1 Sequence Header OBU), preserved verbatim.
    pub config_obus: Vec<u8>,
}

impl Av1ConfigurationBox {
    /// Bit depth in bits (8, 10, or 12) derived from `high_bitdepth`/`twelve_bit`.
    pub fn bit_depth(&self) -> u8 {
        if self.twelve_bit {
            12
        } else if self.high_bitdepth {
            10
        } else {
            8
        }
    }

    /// RFC 6381 codec string `av01.P.LLT.DD` (short form; colour parameters,
    /// which default to `.01.01.01.0`, are omitted per the spec's omittable-tail
    /// rule). `P`=profile, `LL`=two-digit level, `T`=tier (`M`/`H`), `DD`=bit depth.
    pub fn rfc6381(&self) -> alloc::string::String {
        use alloc::format;
        let tier = if self.seq_tier_0 { 'H' } else { 'M' };
        format!(
            "av01.{}.{:02}{}.{:02}",
            self.seq_profile,
            self.seq_level_idx_0,
            tier,
            self.bit_depth()
        )
    }
}

impl<'a> Parse<'a> for Av1ConfigurationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < AV1C_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: AV1C_FIXED_LEN,
                have: bytes.len(),
                what: "av1C body",
            });
        }
        let b0 = bytes[0];
        if (b0 & MARKER_BIT) == 0 {
            return Err(Error::InvalidValue {
                field: "av1C marker",
                value: b0 as u64,
                reason: "marker bit must be 1",
            });
        }
        let version = b0 & 0x7F;
        let b1 = bytes[1];
        let seq_profile = b1 >> 5;
        let seq_level_idx_0 = b1 & 0x1F;
        let b2 = bytes[2];
        let seq_tier_0 = (b2 & 0x80) != 0;
        let high_bitdepth = (b2 & 0x40) != 0;
        let twelve_bit = (b2 & 0x20) != 0;
        let monochrome = (b2 & 0x10) != 0;
        let chroma_subsampling_x = (b2 & 0x08) != 0;
        let chroma_subsampling_y = (b2 & 0x04) != 0;
        let chroma_sample_position = b2 & 0x03;
        let b3 = bytes[3];
        let ipd_present = (b3 & 0x10) != 0;
        let initial_presentation_delay_minus_one = if ipd_present { Some(b3 & 0x0F) } else { None };
        let config_obus = bytes[AV1C_FIXED_LEN..].to_vec();
        Ok(Self {
            version,
            seq_profile,
            seq_level_idx_0,
            seq_tier_0,
            high_bitdepth,
            twelve_bit,
            monochrome,
            chroma_subsampling_x,
            chroma_subsampling_y,
            chroma_sample_position,
            initial_presentation_delay_minus_one,
            config_obus,
        })
    }
}

impl Serialize for Av1ConfigurationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        AV1C_FIXED_LEN + self.config_obus.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = MARKER_BIT | (self.version & 0x7F);
        buf[1] = (self.seq_profile << 5) | (self.seq_level_idx_0 & 0x1F);
        buf[2] = ((self.seq_tier_0 as u8) << 7)
            | ((self.high_bitdepth as u8) << 6)
            | ((self.twelve_bit as u8) << 5)
            | ((self.monochrome as u8) << 4)
            | ((self.chroma_subsampling_x as u8) << 3)
            | ((self.chroma_subsampling_y as u8) << 2)
            | (self.chroma_sample_position & 0x03);
        buf[3] = match self.initial_presentation_delay_minus_one {
            Some(d) => 0x10 | (d & 0x0F),
            None => 0x00,
        };
        buf[AV1C_FIXED_LEN..need].copy_from_slice(&self.config_obus);
        Ok(need)
    }
}

/// AV1 sample entry (`av01`) — a `VisualSampleEntry` carrying an `av1C` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Av1SampleEntry {
    /// Fixed VisualSampleEntry fields.
    pub visual: VisualSampleEntryFields,
    /// The `av1C` configuration box.
    pub config: Av1ConfigurationBox,
}

impl Av1SampleEntry {
    /// Parse from full box bytes (including the 8-byte header).
    pub fn parse_entry(bytes: &[u8]) -> Result<Self> {
        let visual = VisualSampleEntryFields::parse_body(bytes, "av01")?;
        let region = &bytes[BOX_HDR + VisualSampleEntryFields::serialized_len()..];
        let av1c = crate::sample_entries::find_config_box(region, &AV1C_FOURCC).ok_or(
            Error::BufferTooShort {
                need: 0,
                have: 0,
                what: "av01 missing av1C",
            },
        )?;
        let config = Av1ConfigurationBox::parse(&av1c[BOX_HDR..])?;
        Ok(Self { visual, config })
    }
}

impl Serialize for Av1SampleEntry {
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
        buf[c..c + 4].copy_from_slice(&AV01_FOURCC);
        c += 4;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        let av1c_len = BOX_HDR + self.config.serialized_len();
        buf[c..c + 4].copy_from_slice(&(av1c_len as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&AV1C_FOURCC);
        c += 4;
        c += self.config.serialize_into(&mut buf[c..])?;
        Ok(c)
    }
}
