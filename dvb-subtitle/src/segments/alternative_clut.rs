//! Alternative CLUT Segment — ETSI EN 300 743 §7.2.8, Table 31 (segment_type 0x16).
//!
//! Permits a CLUT to be defined in colour systems other than ITU-R BT.601.

use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// The alternative_CLUT_segment segment_type.
pub const SEGMENT_TYPE: u8 = 0x16;
/// Header: 6 bytes.
pub const HEADER_LEN: usize = 6;
/// Fixed body: CLUT_id(1) + CLUT_version_number(4b)+reserved(4b) + CLUT_parameters(2) = 4 bytes.
pub const FIXED_LEN: usize = 4;
/// Entry for 8-bit output: 4 bytes.
pub const ENTRY_8BIT_LEN: usize = 4;
/// Entry for 10-bit output: 5 bytes.
pub const ENTRY_10BIT_LEN: usize = 5;

/// CLUT parameters as defined in Table 32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ClutParameters {
    /// Maximum number of CLUT entries (0 = 256).
    pub entry_max_number: u8,
    /// Colour component type (0 = YCbCr).
    pub colour_component_type: u8,
    /// Output bit depth (0 = 8-bit, 1 = 10-bit).
    pub output_bit_depth: u8,
    /// Reserved bit in CLUT_parameters byte 0 (bit 0).
    pub reserved: u8,
    /// SDR/HDR dynamic range and colour gamut as per Table 34.
    pub dynamic_range_and_colour_gamut: u8,
}

impl ClutParameters {
    /// Parse CLUT_parameters from 2 raw bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: bytes.len(),
                what: "CLUT_parameters",
            });
        }
        let entry_max_number = bytes[0] >> 6;
        let colour_component_type = (bytes[0] >> 4) & 0x03;
        let output_bit_depth = (bytes[0] >> 1) & 0x07;
        let reserved = bytes[0] & 0x01;
        Ok(ClutParameters {
            entry_max_number,
            colour_component_type,
            output_bit_depth,
            reserved,
            dynamic_range_and_colour_gamut: bytes[1],
        })
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        buf[0] = (self.entry_max_number << 6)
            | (self.colour_component_type << 4)
            | (self.output_bit_depth << 1)
            | (self.reserved & 0x01);
        buf[1] = self.dynamic_range_and_colour_gamut;
    }
}

/// Output bit depth as defined in Table 33.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum OutputBitDepth {
    /// 8-bit per component.
    Bit8 = 0x00,
    /// 10-bit per component.
    Bit10 = 0x01,
    /// Reserved.
    Reserved(u8),
}

impl OutputBitDepth {
    /// Human-readable name for this bit depth.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Bit8 => "8-bit",
            Self::Bit10 => "10-bit",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(OutputBitDepth, Reserved);

/// Dynamic range and colour gamut as defined in Table 34.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum DynamicRangeColourGamut {
    /// SDR; ITU-R BT.709.
    SdrBt709 = 0x00,
    /// SDR; ITU-R BT.2020-2.
    SdrBt2020 = 0x01,
    /// HDR; ITU-R BT.2100-1 PQ.
    HdrBt2100Pq = 0x02,
    /// HDR; ITU-R BT.2100-1 HLG.
    HdrBt2100Hlg = 0x03,
    /// Reserved.
    Reserved(u8),
}

impl DynamicRangeColourGamut {
    /// Human-readable name for this dynamic range and colour gamut.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SdrBt709 => "SDR_BT.709",
            Self::SdrBt2020 => "SDR_BT.2020",
            Self::HdrBt2100Pq => "HDR_BT.2100_PQ",
            Self::HdrBt2100Hlg => "HDR_BT.2100_HLG",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(DynamicRangeColourGamut, Reserved);

/// A single alternative CLUT entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AlternativeClutEntry {
    /// Luma value (8 or 10 bit).
    pub luma_value: u16,
    /// Chroma1 (Cb) value (8 or 10 bit).
    pub chroma1_value: u16,
    /// Chroma2 (Cr) value (8 or 10 bit).
    pub chroma2_value: u16,
    /// Transparency value (8 or 10 bit).
    pub t_value: u16,
}

impl AlternativeClutEntry {
    fn serialized_len(output_bit_depth: u8) -> usize {
        match output_bit_depth {
            0 => ENTRY_8BIT_LEN,
            1 => ENTRY_10BIT_LEN,
            _ => ENTRY_8BIT_LEN,
        }
    }

    fn serialize_into(&self, buf: &mut [u8], output_bit_depth: u8) {
        if output_bit_depth == 1 {
            let v0 = self.luma_value;
            let v1 = self.chroma1_value;
            let v2 = self.chroma2_value;
            let v3 = self.t_value;
            buf[0] = (v0 >> 2) as u8;
            buf[1] = (((v0 & 0x03) << 6) | ((v1 & 0x3FC) >> 4)) as u8;
            buf[2] = (((v1 & 0x0F) << 4) | ((v2 & 0x3F0) >> 6)) as u8;
            buf[3] = (((v2 & 0x3F) << 2) | ((v3 & 0x300) >> 8)) as u8;
            buf[4] = (v3 & 0xFF) as u8;
        } else {
            buf[0] = self.luma_value as u8;
            buf[1] = self.chroma1_value as u8;
            buf[2] = self.chroma2_value as u8;
            buf[3] = self.t_value as u8;
        }
    }

    fn parse(bytes: &[u8], output_bit_depth: u8) -> Result<Self> {
        let entry_len = Self::serialized_len(output_bit_depth);
        if bytes.len() < entry_len {
            return Err(Error::BufferTooShort {
                need: entry_len,
                have: bytes.len(),
                what: "alternative_CLUT_entry",
            });
        }
        let (luma, c1, c2, t) = if output_bit_depth == 1 {
            let v0 = ((bytes[0] as u16) << 2) | ((bytes[1] as u16) >> 6);
            let v1 = (((bytes[1] as u16) & 0x3F) << 4) | ((bytes[2] as u16) >> 4);
            let v2 = (((bytes[2] as u16) & 0x0F) << 6) | ((bytes[3] as u16) >> 2);
            let v3 = (((bytes[3] as u16) & 0x03) << 8) | (bytes[4] as u16);
            (v0, v1, v2, v3)
        } else {
            (
                bytes[0] as u16,
                bytes[1] as u16,
                bytes[2] as u16,
                bytes[3] as u16,
            )
        };
        Ok(AlternativeClutEntry {
            luma_value: luma,
            chroma1_value: c1,
            chroma2_value: c2,
            t_value: t,
        })
    }
}

/// Alternative CLUT Segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AlternativeClutSegment {
    /// The page_id from the segment header.
    pub page_id: u16,
    /// CLUT family identifier.
    pub clut_id: u8,
    /// CLUT version number (modulo 16).
    pub clut_version_number: u8,
    /// Reserved bits in body byte 1 (bits `[3:0]`).
    pub reserved: u8,
    /// CLUT parameters.
    pub clut_parameters: ClutParameters,
    /// CLUT entries.
    pub entries: alloc::vec::Vec<AlternativeClutEntry>,
}

impl<'a> Parse<'a> for AlternativeClutSegment {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + FIXED_LEN,
                have: bytes.len(),
                what: "alternative_CLUT_segment",
            });
        }
        if bytes[1] != SEGMENT_TYPE {
            return Err(Error::UnknownSegmentType(bytes[1]));
        }
        let page_id = u16::from_be_bytes([bytes[2], bytes[3]]);
        let segment_length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let total = HEADER_LEN + segment_length;
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "alternative_CLUT_segment data",
            });
        }
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        if body.len() < FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN,
                have: body.len(),
                what: "alternative_CLUT_segment body",
            });
        }
        let clut_id = body[0];
        let clut_version_number = body[1] >> 4;
        let reserved = body[1] & 0x0F;
        let clut_parameters = ClutParameters::parse(&body[2..4])?;
        let output_bit_depth = clut_parameters.output_bit_depth;
        let entry_len = AlternativeClutEntry::serialized_len(output_bit_depth);

        let entry_data = &body[FIXED_LEN..];
        let num_entries = entry_data.len() / entry_len;
        let mut entries = alloc::vec::Vec::with_capacity(num_entries);
        for i in 0..num_entries {
            let off = i * entry_len;
            entries.push(AlternativeClutEntry::parse(
                &entry_data[off..off + entry_len],
                output_bit_depth,
            )?);
        }

        Ok(AlternativeClutSegment {
            page_id,
            clut_id,
            clut_version_number,
            reserved,
            clut_parameters,
            entries,
        })
    }
}

impl Serialize for AlternativeClutSegment {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let entry_len = AlternativeClutEntry::serialized_len(self.clut_parameters.output_bit_depth);
        HEADER_LEN + FIXED_LEN + self.entries.len() * entry_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "alternative_CLUT_segment serialize",
            });
        }
        buf[0] = 0x0F;
        buf[1] = SEGMENT_TYPE;
        buf[2..4].copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = (len - HEADER_LEN) as u16;
        buf[4..6].copy_from_slice(&seg_len.to_be_bytes());

        buf[6] = self.clut_id;
        buf[7] = (self.clut_version_number << 4) | (self.reserved & 0x0F);
        self.clut_parameters.serialize_into(&mut buf[8..10]);

        let entry_len = AlternativeClutEntry::serialized_len(self.clut_parameters.output_bit_depth);
        for (i, entry) in self.entries.iter().enumerate() {
            let off = HEADER_LEN + FIXED_LEN + i * entry_len;
            entry.serialize_into(
                &mut buf[off..off + entry_len],
                self.clut_parameters.output_bit_depth,
            );
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    #[test]
    fn round_trip_8bit() {
        let bytes = [
            0x0F, 0x16, 0x00, 0x01, 0x00, 0x08, 0x03, 0x10, 0x00, 0x00, 0x80, 0x80, 0x80, 0x80,
        ];
        let seg = AlternativeClutSegment::parse(&bytes).unwrap();
        assert_eq!(seg.clut_id, 3);
        assert_eq!(seg.entries.len(), 1);
        assert_eq!(seg.entries[0].luma_value, 128);
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.entries[0].luma_value = 200;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = AlternativeClutSegment::parse(&out2).unwrap();
        assert_eq!(reparse.entries[0].luma_value, 200);
    }

    #[test]
    fn round_trip_10bit() {
        let bytes = [
            0x0F, 0x16, 0x00, 0x01, 0x00, 0x09, 0x03, 0x10, 0x02, 0x02, 0x80, 0x00, 0x00, 0x00,
            0x00,
        ];
        let seg = AlternativeClutSegment::parse(&bytes).unwrap();
        assert_eq!(seg.clut_parameters.output_bit_depth, 1);
        assert_eq!(seg.entries.len(), 1);
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.clut_id = 7;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = AlternativeClutSegment::parse(&out2).unwrap();
        assert_eq!(reparse.clut_id, 7);
    }
}
