//! AV1 codec configuration box in ISOBMFF — the `av1C` record.
//!
//! AOMedia "AV1 Codec ISO Media File Format Binding"
//! (<https://aomediacodec.github.io/av1-isobmff/>, §2.3 AV1CodecConfigurationBox).
//!
//! | Box | FourCC | Description |
//! |-----|--------|-------------|
//! | [`Av1ConfigurationBox`] | `av1C` | AV1CodecConfigurationRecord |
//!
//! The record's `configOBUs` tail carries an optional Sequence Header OBU verbatim;
//! it is preserved opaquely so the box round-trips byte-exact.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::error::InvalidValue;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// FourCC of the AV1 config box.
pub const AV1C_FOURCC: [u8; 4] = *b"av1C";
/// Fixed size of the `av1C` record before the trailing `configOBUs`.
const AV1C_FIXED_LEN: usize = 4;
/// `marker(1)=1` in the high bit of byte 0.
const MARKER_BIT: u8 = 0x80;
/// Bit depths the binding signals (`high_bitdepth`/`twelve_bit`).
const BIT_DEPTH_STANDARD: u8 = 8;
const BIT_DEPTH_HIGH: u8 = 10;
const BIT_DEPTH_MAXIMUM: u8 = 12;
/// Byte offsets of the `av1C` fixed-record fields.
const VERSION_OFFSET: usize = 0;
const SEQ_PROFILE_OFFSET: usize = 1;
const CHROMA_OFFSET: usize = 2;
const IPD_OFFSET: usize = 3;
/// `version` mask after the marker bit.
const VERSION_MASK: u8 = 0x7F;
/// Shift and mask of `seq_profile` in byte 1.
const SEQ_PROFILE_SHIFT: u32 = 5;
const SEQ_PROFILE_MASK: u8 = 0x07;
/// Mask of `seq_level_idx_0` in byte 1.
const SEQ_LEVEL_IDX_MASK: u8 = 0x1F;
/// Bit shifts and masks of the byte-2 fields.
const SEQ_TIER_SHIFT: u32 = 7;
const SEQ_TIER_MASK: u8 = 0x80;
const HIGH_BITDEPTH_SHIFT: u32 = 6;
const HIGH_BITDEPTH_MASK: u8 = 0x40;
const FULL_BITDEPTH_FLAG_SHIFT: u32 = 5;
const FULL_BITDEPTH_FLAG_MASK: u8 = 0x20;
const MONOCHROME_SHIFT: u32 = 4;
const MONOCHROME_MASK: u8 = 0x10;
const CHROMA_SUBSAMPLING_X_SHIFT: u32 = 3;
const CHROMA_SUBSAMPLING_X_MASK: u8 = 0x08;
const CHROMA_SUBSAMPLING_Y_SHIFT: u32 = 2;
const CHROMA_SUBSAMPLING_Y_MASK: u8 = 0x04;
const CHROMA_SAMPLE_POSITION_MASK: u8 = 0x03;
/// Bit and mask of `initial_presentation_delay_present`/`_minus_one`.
const IPD_PRESENT_MASK: u8 = 0x10;
const IPD_DELAY_MASK: u8 = 0x0F;

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
            BIT_DEPTH_MAXIMUM
        } else if self.high_bitdepth {
            BIT_DEPTH_HIGH
        } else {
            BIT_DEPTH_STANDARD
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
        need_in(bytes, AV1C_FIXED_LEN, "av1C body")?;
        let b0 = bytes[VERSION_OFFSET];
        if (b0 & MARKER_BIT) == 0 {
            return Err(Error::InvalidValue(InvalidValue {
                field: "av1C marker",
                value: b0 as u64,
                reason: "marker bit must be 1",
            }));
        }
        let version = b0 & VERSION_MASK;
        let b1 = bytes[SEQ_PROFILE_OFFSET];
        let seq_profile = (b1 >> SEQ_PROFILE_SHIFT) & SEQ_PROFILE_MASK;
        let seq_level_idx_0 = b1 & SEQ_LEVEL_IDX_MASK;
        let b2 = bytes[CHROMA_OFFSET];
        let seq_tier_0 = (b2 & SEQ_TIER_MASK) != 0;
        let high_bitdepth = (b2 & HIGH_BITDEPTH_MASK) != 0;
        let twelve_bit = (b2 & FULL_BITDEPTH_FLAG_MASK) != 0;
        let monochrome = (b2 & MONOCHROME_MASK) != 0;
        let chroma_subsampling_x = (b2 & CHROMA_SUBSAMPLING_X_MASK) != 0;
        let chroma_subsampling_y = (b2 & CHROMA_SUBSAMPLING_Y_MASK) != 0;
        let chroma_sample_position = b2 & CHROMA_SAMPLE_POSITION_MASK;
        let b3 = bytes[IPD_OFFSET];
        let ipd_present = (b3 & IPD_PRESENT_MASK) != 0;
        let initial_presentation_delay_minus_one = if ipd_present {
            Some(b3 & IPD_DELAY_MASK)
        } else {
            None
        };
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
        need_out(buf, need)?;
        buf[VERSION_OFFSET] = MARKER_BIT | (self.version & VERSION_MASK);
        buf[SEQ_PROFILE_OFFSET] = ((self.seq_profile & SEQ_PROFILE_MASK) << SEQ_PROFILE_SHIFT)
            | (self.seq_level_idx_0 & SEQ_LEVEL_IDX_MASK);
        buf[CHROMA_OFFSET] = ((self.seq_tier_0 as u8) << SEQ_TIER_SHIFT)
            | ((self.high_bitdepth as u8) << HIGH_BITDEPTH_SHIFT)
            | ((self.twelve_bit as u8) << FULL_BITDEPTH_FLAG_SHIFT)
            | ((self.monochrome as u8) << MONOCHROME_SHIFT)
            | ((self.chroma_subsampling_x as u8) << CHROMA_SUBSAMPLING_X_SHIFT)
            | ((self.chroma_subsampling_y as u8) << CHROMA_SUBSAMPLING_Y_SHIFT)
            | (self.chroma_sample_position & CHROMA_SAMPLE_POSITION_MASK);
        buf[IPD_OFFSET] = match self.initial_presentation_delay_minus_one {
            Some(d) => IPD_PRESENT_MASK | (d & IPD_DELAY_MASK),
            None => 0,
        };
        buf[AV1C_FIXED_LEN..need].copy_from_slice(&self.config_obus);
        Ok(need)
    }
}
