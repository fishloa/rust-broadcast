//! ISOBMFF `mdcv` / `clli` box bodies — ISO/IEC 14496-12:2026 §12.1.7 and
//! §12.1.6.
//!
//! The same byte layouts are the H.265 `mastering_display_colour_volume`
//! (D.2.28) and `content_light_level_info` (D.2.35) SEI payloads. The SEI
//! carrier that absorbs and emits them lives in `broadcast_bitstream::hdr`.

use crate::error::{Error, Result};
use broadcast_core::error::{BufferTooShort, InvalidValue};
use broadcast_core::len::need_out;
use broadcast_core::{Parse, Serialize};
use core::mem::size_of;

/// FourCC of the mastering display colour volume box.
pub const MDCV_FOURCC: [u8; 4] = *b"mdcv";
/// FourCC of the content light level box.
pub const CLLI_FOURCC: [u8; 4] = *b"clli";

const CIE_PRIMARY_COUNT: usize = 3;
pub const MDCV_PAYLOAD_LEN: usize = 24;
pub const CLLI_PAYLOAD_LEN: usize = 4;

/// Width of a box type / four-character code in bytes.
pub const BOX_TYPE_LEN: usize = 4;
/// A `ChromaticityXy` pair is two 16-bit coordinates (H.265 D.2.28).
const CHROMATICITY_LEN: usize = 2;
/// Stride of one `display_primaries` entry: `x` then `y`.
const PRIMARY_STRIDE: usize = CHROMATICITY_LEN * 2;
/// Offset of `display_primaries_y[c]` inside its entry.
const PRIMARY_Y_OFFSET: usize = CHROMATICITY_LEN;
/// Offset of `white_point_y` after `white_point_x`.
const WHITE_POINT_Y_OFFSET: usize = CHROMATICITY_LEN;
/// Offset of `max_display_mastering_luminance` after the white point.
const MAX_MASTERING_LUMINANCE_OFFSET: usize = WHITE_POINT_Y_OFFSET + CHROMATICITY_LEN;
/// Offset of `min_display_mastering_luminance` after the white point.
const MIN_MASTERING_LUMINANCE_OFFSET: usize = MAX_MASTERING_LUMINANCE_OFFSET + size_of::<u32>();
/// Offset of `max_pic_average_light_level` after `max_content_light_level`.
const MAX_PIC_AVERAGE_LIGHT_LEVEL_OFFSET: usize = CHROMATICITY_LEN;

/// A CIE 1931 chromaticity coordinate pair in increments of 0.00002 (H.265
/// D.3.28; 50000 = 1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromaticityXy {
    /// `x`, 0..=50000.
    pub x: u16,
    /// `y`, 0..=50000.
    pub y: u16,
}

/// `mastering_display_colour_volume` (H.265 D.2.28) / ISOBMFF `mdcv` body.
///
/// `display_primaries` are in SEI order: index 0 green, 1 blue, 2 red.
/// Luminances are in units of 0.0001 cd/m2 (H.265 D.3.28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MasteringDisplayColourVolume {
    /// `display_primaries_x[c]` / `display_primaries_y[c]` for c = 0..3.
    pub display_primaries: [ChromaticityXy; CIE_PRIMARY_COUNT],
    /// `white_point_x` / `white_point_y`.
    pub white_point: ChromaticityXy,
    /// `max_display_mastering_luminance`.
    pub max_display_mastering_luminance: u32,
    /// `min_display_mastering_luminance`.
    pub min_display_mastering_luminance: u32,
}

/// `content_light_level_info` (H.265 D.2.35) / ISOBMFF `clli` body, in cd/m2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentLightLevel {
    /// `max_content_light_level` (MaxCLL).
    pub max_content_light_level: u16,
    /// `max_pic_average_light_level` (MaxFALL).
    pub max_pic_average_light_level: u16,
}

fn be16(b: &[u8], at: usize) -> u16 {
    let mut word = [0u8; size_of::<u16>()];
    word.copy_from_slice(&b[at..at + size_of::<u16>()]);
    u16::from_be_bytes(word)
}

fn be32(b: &[u8], at: usize) -> u32 {
    let mut word = [0u8; size_of::<u32>()];
    word.copy_from_slice(&b[at..at + size_of::<u32>()]);
    u32::from_be_bytes(word)
}

fn check_exact(bytes: &[u8], need: usize, what: &'static str) -> Result<()> {
    if bytes.len() < need {
        return Err(Error::BufferTooShort(BufferTooShort {
            need,
            have: bytes.len(),
            what,
        }));
    }
    if bytes.len() > need {
        return Err(Error::InvalidValue(InvalidValue {
            field: what,
            value: bytes.len() as u64,
            reason: "payload longer than the defined syntax",
        }));
    }
    Ok(())
}

fn check_out(buf: &[u8], need: usize) -> Result<()> {
    need_out(buf, need)?;
    Ok(())
}

impl<'a> Parse<'a> for MasteringDisplayColourVolume {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        check_exact(bytes, MDCV_PAYLOAD_LEN, "mdcv payload")?;
        let mut display_primaries = [ChromaticityXy { x: 0, y: 0 }; CIE_PRIMARY_COUNT];
        for (c, p) in display_primaries.iter_mut().enumerate() {
            p.x = be16(bytes, c * PRIMARY_STRIDE);
            p.y = be16(bytes, c * PRIMARY_STRIDE + PRIMARY_Y_OFFSET);
        }
        let wp_at = CIE_PRIMARY_COUNT * PRIMARY_STRIDE;
        Ok(Self {
            display_primaries,
            white_point: ChromaticityXy {
                x: be16(bytes, wp_at),
                y: be16(bytes, wp_at + WHITE_POINT_Y_OFFSET),
            },
            max_display_mastering_luminance: be32(bytes, wp_at + MAX_MASTERING_LUMINANCE_OFFSET),
            min_display_mastering_luminance: be32(bytes, wp_at + MIN_MASTERING_LUMINANCE_OFFSET),
        })
    }
}

impl Serialize for MasteringDisplayColourVolume {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        MDCV_PAYLOAD_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        check_out(buf, MDCV_PAYLOAD_LEN)?;
        for (c, p) in self.display_primaries.iter().enumerate() {
            let at = c * PRIMARY_STRIDE;
            buf[at..at + CHROMATICITY_LEN].copy_from_slice(&p.x.to_be_bytes());
            buf[at + PRIMARY_Y_OFFSET..at + PRIMARY_STRIDE].copy_from_slice(&p.y.to_be_bytes());
        }
        let wp_at = CIE_PRIMARY_COUNT * PRIMARY_STRIDE;
        buf[wp_at..wp_at + CHROMATICITY_LEN].copy_from_slice(&self.white_point.x.to_be_bytes());
        buf[wp_at + WHITE_POINT_Y_OFFSET..wp_at + PRIMARY_STRIDE]
            .copy_from_slice(&self.white_point.y.to_be_bytes());
        buf[wp_at + MAX_MASTERING_LUMINANCE_OFFSET..wp_at + MIN_MASTERING_LUMINANCE_OFFSET]
            .copy_from_slice(&self.max_display_mastering_luminance.to_be_bytes());
        buf[wp_at + MIN_MASTERING_LUMINANCE_OFFSET
            ..wp_at + MIN_MASTERING_LUMINANCE_OFFSET + size_of::<u32>()]
            .copy_from_slice(&self.min_display_mastering_luminance.to_be_bytes());
        Ok(MDCV_PAYLOAD_LEN)
    }
}

impl<'a> Parse<'a> for ContentLightLevel {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        check_exact(bytes, CLLI_PAYLOAD_LEN, "clli payload")?;
        Ok(Self {
            max_content_light_level: be16(bytes, 0),
            max_pic_average_light_level: be16(bytes, MAX_PIC_AVERAGE_LIGHT_LEVEL_OFFSET),
        })
    }
}

impl Serialize for ContentLightLevel {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        CLLI_PAYLOAD_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        check_out(buf, CLLI_PAYLOAD_LEN)?;
        buf[..2].copy_from_slice(&self.max_content_light_level.to_be_bytes());
        buf[2..4].copy_from_slice(&self.max_pic_average_light_level.to_be_bytes());
        Ok(CLLI_PAYLOAD_LEN)
    }
}
