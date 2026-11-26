//! Alternative CLUT Segment — ETSI EN 300 743 §7.2.8, Table 31 (segment_type 0x16).
//!
//! Permits a CLUT to be defined in colour systems other than ITU-R BT.601.

use crate::error::{Error, Result};
use crate::segments::{
    PAGE_ID_LEN, PAGE_ID_OFFSET, SEGMENT_LENGTH_LEN, SEGMENT_LENGTH_OFFSET, SEGMENT_TYPE_OFFSET,
    be_u16,
};
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

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

/// Bytes of the CLUT_parameters field.
const CLUT_PARAMETERS_LEN: usize = 2;

/// Shifts and masks of CLUT_parameters byte 0 (Table 32).
const ENTRY_MAX_NUMBER_SHIFT: u32 = 6;
const ENTRY_MAX_NUMBER_MASK: u8 = 0x03;
const COLOUR_COMPONENT_TYPE_SHIFT: u32 = 4;
const COLOUR_COMPONENT_TYPE_MASK: u8 = 0x03;
const OUTPUT_BIT_DEPTH_SHIFT: u32 = 1;
const OUTPUT_BIT_DEPTH_MASK: u8 = 0x07;
const PARAMETERS_RFU_MASK: u8 = 0x01;
/// `output_bit_depth` code selecting 10-bit components.
const OUTPUT_BIT_DEPTH_10: u8 = 1;

/// Mask of a 10-bit colour component.
const ENTRY_10BIT_MASK: u16 = 0x03FF;
/// Mask of an 8-bit colour component.
const ENTRY_8BIT_MASK: u16 = 0x00FF;

/// Shifts and masks packing four 10-bit values into five bytes.
const ENTRY_10BIT_SEGMENT_SHIFT: u32 = 2;
const ENTRY_10BIT_SEGMENT_MASK: u16 = 0x0003;
const ENTRY_10BIT_PAIR_HIGH_SHIFT: u32 = 6;
const ENTRY_10BIT_CHROMA1_MASK: u16 = 0x03FC;
const ENTRY_10BIT_CHROMA1_SHIFT: u32 = 4;
const ENTRY_10BIT_CHROMA1_LOW_MASK: u16 = 0x000F;
const ENTRY_10BIT_CHROMA2_MASK: u16 = 0x03F0;
const ENTRY_10BIT_CHROMA2_SHIFT: u32 = 6;
const ENTRY_10BIT_CHROMA2_LOW_MASK: u16 = 0x003F;
const ENTRY_10BIT_T_LOW_SHIFT: u32 = 8;
const ENTRY_10BIT_T_MASK: u16 = 0x0300;
const ENTRY_10BIT_BYTE_MASK: u16 = 0x00FF;

/// Offsets within a 10-bit entry's packed bytes.
const ENTRY_10BIT_LUMA_OFFSET: usize = 0;
const ENTRY_10BIT_CHROMA1_OFFSET: usize = 1;
const ENTRY_10BIT_CHROMA2_OFFSET: usize = 2;
const ENTRY_10BIT_T_OFFSET: usize = 3;
const ENTRY_10BIT_LAST_OFFSET: usize = 4;
/// Offsets within an 8-bit entry's packed bytes.
const ENTRY_8BIT_CHROMA1_OFFSET: usize = 1;
const ENTRY_8BIT_CHROMA2_OFFSET: usize = 2;
const ENTRY_8BIT_T_OFFSET: usize = 3;

/// Offset of `CLUT_id` within the segment body.
const CLUT_ID_OFFSET: usize = 0;
/// Offset of the CLUT version/reserved byte within the segment body.
const CLUT_VERSION_OFFSET: usize = 1;
/// Offset of the CLUT_parameters field within the segment body.
const CLUT_PARAMETERS_OFFSET: usize = 2;
/// Shift placing `clut_version_number` above the reserved nibble.
const CLUT_VERSION_SHIFT: u32 = 4;
/// Mask of the reserved nibble of the version byte.
const CLUT_VERSION_RFU_MASK: u8 = 0x0F;

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
        need_in(bytes, CLUT_PARAMETERS_LEN, "CLUT_parameters")?;
        let entry_max_number = bytes[0] >> ENTRY_MAX_NUMBER_SHIFT;
        let colour_component_type =
            (bytes[0] >> COLOUR_COMPONENT_TYPE_SHIFT) & COLOUR_COMPONENT_TYPE_MASK;
        let output_bit_depth = (bytes[0] >> OUTPUT_BIT_DEPTH_SHIFT) & OUTPUT_BIT_DEPTH_MASK;
        let reserved = bytes[0] & PARAMETERS_RFU_MASK;
        Ok(ClutParameters {
            entry_max_number,
            colour_component_type,
            output_bit_depth,
            reserved,
            dynamic_range_and_colour_gamut: bytes[1],
        })
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<()> {
        buf[0] = (broadcast_core::len::fit_mask(
            self.entry_max_number,
            ENTRY_MAX_NUMBER_MASK,
            "entry_max_number",
        )? << ENTRY_MAX_NUMBER_SHIFT)
            | (broadcast_core::len::fit_mask(
                self.colour_component_type,
                COLOUR_COMPONENT_TYPE_MASK,
                "colour_component_type",
            )? << COLOUR_COMPONENT_TYPE_SHIFT)
            | (broadcast_core::len::fit_mask(
                self.output_bit_depth,
                OUTPUT_BIT_DEPTH_MASK,
                "output_bit_depth",
            )? << OUTPUT_BIT_DEPTH_SHIFT)
            | broadcast_core::len::fit_mask(self.reserved, PARAMETERS_RFU_MASK, "reserved")?;
        buf[1] = self.dynamic_range_and_colour_gamut;
        Ok(())
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

broadcast_core::impl_spec_display!(OutputBitDepth, Reserved);

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

broadcast_core::impl_spec_display!(DynamicRangeColourGamut, Reserved);

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

    fn serialize_into(&self, buf: &mut [u8], output_bit_depth: u8) -> Result<()> {
        if output_bit_depth == OUTPUT_BIT_DEPTH_10 {
            // 10-bit entry: each of the four values is 10 bits wide.
            let v0 =
                broadcast_core::len::fit_mask(self.luma_value, ENTRY_10BIT_MASK, "luma_value")?;
            let v1 = broadcast_core::len::fit_mask(
                self.chroma1_value,
                ENTRY_10BIT_MASK,
                "chroma1_value",
            )?;
            let v2 = broadcast_core::len::fit_mask(
                self.chroma2_value,
                ENTRY_10BIT_MASK,
                "chroma2_value",
            )?;
            let v3 = broadcast_core::len::fit_mask(self.t_value, ENTRY_10BIT_MASK, "t_value")?;
            buf[ENTRY_10BIT_LUMA_OFFSET] = (v0 >> ENTRY_10BIT_SEGMENT_SHIFT) as u8;
            buf[ENTRY_10BIT_CHROMA1_OFFSET] = (((v0 & ENTRY_10BIT_SEGMENT_MASK)
                << ENTRY_10BIT_PAIR_HIGH_SHIFT)
                | ((v1 & ENTRY_10BIT_CHROMA1_MASK) >> ENTRY_10BIT_CHROMA1_SHIFT))
                as u8;
            buf[ENTRY_10BIT_CHROMA2_OFFSET] = (((v1 & ENTRY_10BIT_CHROMA1_LOW_MASK)
                << ENTRY_10BIT_CHROMA1_SHIFT)
                | ((v2 & ENTRY_10BIT_CHROMA2_MASK) >> ENTRY_10BIT_CHROMA2_SHIFT))
                as u8;
            buf[ENTRY_10BIT_T_OFFSET] =
                (((v2 & ENTRY_10BIT_CHROMA2_LOW_MASK) << ENTRY_10BIT_SEGMENT_SHIFT)
                    | ((v3 & ENTRY_10BIT_T_MASK) >> ENTRY_10BIT_T_LOW_SHIFT)) as u8;
            buf[ENTRY_10BIT_LAST_OFFSET] = (v3 & ENTRY_10BIT_BYTE_MASK) as u8;
        } else {
            // 8-bit entry: each value is 8 bits wide.
            buf[ENTRY_10BIT_LUMA_OFFSET] =
                broadcast_core::len::fit_mask(self.luma_value, ENTRY_8BIT_MASK, "luma_value")?
                    as u8;
            buf[ENTRY_8BIT_CHROMA1_OFFSET] = broadcast_core::len::fit_mask(
                self.chroma1_value,
                ENTRY_8BIT_MASK,
                "chroma1_value",
            )? as u8;
            buf[ENTRY_8BIT_CHROMA2_OFFSET] = broadcast_core::len::fit_mask(
                self.chroma2_value,
                ENTRY_8BIT_MASK,
                "chroma2_value",
            )? as u8;
            buf[ENTRY_8BIT_T_OFFSET] =
                broadcast_core::len::fit_mask(self.t_value, ENTRY_8BIT_MASK, "t_value")? as u8;
        }
        Ok(())
    }

    fn parse(bytes: &[u8], output_bit_depth: u8) -> Result<Self> {
        let entry_len = Self::serialized_len(output_bit_depth);
        need_in(bytes, entry_len, "alternative_CLUT_entry")?;
        let (luma, c1, c2, t) = if output_bit_depth == OUTPUT_BIT_DEPTH_10 {
            let v0 = (u16::from(bytes[ENTRY_10BIT_LUMA_OFFSET]) << ENTRY_10BIT_SEGMENT_SHIFT)
                | (u16::from(bytes[ENTRY_10BIT_CHROMA1_OFFSET]) >> ENTRY_10BIT_PAIR_HIGH_SHIFT);
            let v1 = ((u16::from(bytes[ENTRY_10BIT_CHROMA1_OFFSET])
                & ENTRY_10BIT_CHROMA2_LOW_MASK)
                << ENTRY_10BIT_CHROMA1_SHIFT)
                | (u16::from(bytes[ENTRY_10BIT_CHROMA2_OFFSET]) >> ENTRY_10BIT_CHROMA1_SHIFT);
            let v2 = ((u16::from(bytes[ENTRY_10BIT_CHROMA2_OFFSET])
                & ENTRY_10BIT_CHROMA1_LOW_MASK)
                << ENTRY_10BIT_CHROMA2_SHIFT)
                | (u16::from(bytes[ENTRY_10BIT_T_OFFSET]) >> ENTRY_10BIT_SEGMENT_SHIFT);
            let v3 = ((u16::from(bytes[ENTRY_10BIT_T_OFFSET]) & ENTRY_10BIT_SEGMENT_MASK)
                << ENTRY_10BIT_T_LOW_SHIFT)
                | u16::from(bytes[ENTRY_10BIT_LAST_OFFSET]);
            (v0, v1, v2, v3)
        } else {
            (
                u16::from(bytes[ENTRY_10BIT_LUMA_OFFSET]),
                u16::from(bytes[ENTRY_8BIT_CHROMA1_OFFSET]),
                u16::from(bytes[ENTRY_8BIT_CHROMA2_OFFSET]),
                u16::from(bytes[ENTRY_8BIT_T_OFFSET]),
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
        need_in(bytes, HEADER_LEN + FIXED_LEN, "alternative_CLUT_segment")?;
        if bytes[0] != crate::pes_data_field::SYNC_BYTE {
            return Err(Error::InvalidSync(broadcast_core::error::InvalidSync {
                what: "sync_byte",
                expected: u32::from(crate::pes_data_field::SYNC_BYTE),
                found: u32::from(bytes[0]),
            }));
        }
        if bytes[SEGMENT_TYPE_OFFSET] != SEGMENT_TYPE {
            return Err(Error::UnknownSegmentType(bytes[SEGMENT_TYPE_OFFSET]));
        }
        let page_id = be_u16(bytes, PAGE_ID_OFFSET);
        let segment_length = be_u16(bytes, SEGMENT_LENGTH_OFFSET) as usize;
        let total = HEADER_LEN + segment_length;
        need_in(bytes, total, "alternative_CLUT_segment data")?;
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        need_in(body, FIXED_LEN, "alternative_CLUT_segment body")?;
        let clut_id = body[CLUT_ID_OFFSET];
        let clut_version_number = body[CLUT_VERSION_OFFSET] >> CLUT_VERSION_SHIFT;
        let reserved = body[CLUT_VERSION_OFFSET] & CLUT_VERSION_RFU_MASK;
        let clut_parameters = ClutParameters::parse(
            &body[CLUT_PARAMETERS_OFFSET..CLUT_PARAMETERS_OFFSET + CLUT_PARAMETERS_LEN],
        )?;
        let output_bit_depth = clut_parameters.output_bit_depth;
        let entry_len = AlternativeClutEntry::serialized_len(output_bit_depth);

        let entry_data = &body[FIXED_LEN..];
        // Table 31's entry loop is `while (processed_length < segment_length)`
        // over whole fixed-size entries — a remainder means the segment is
        // malformed (#1108/DS-W6: pre-fix, integer division silently dropped
        // it instead).
        if !entry_data.len().is_multiple_of(entry_len) {
            return Err(Error::TrailingEntryBytes {
                what: "alternative_CLUT_segment entries",
                entry_len,
                extra: entry_data.len() % entry_len,
            });
        }
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
        need_out(buf, len)?;
        buf[0] = crate::pes_data_field::SYNC_BYTE;
        buf[SEGMENT_TYPE_OFFSET] = SEGMENT_TYPE;
        buf[PAGE_ID_OFFSET..PAGE_ID_OFFSET + PAGE_ID_LEN]
            .copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = crate::segments::check_segment_length(len - HEADER_LEN)?;
        buf[SEGMENT_LENGTH_OFFSET..SEGMENT_LENGTH_OFFSET + SEGMENT_LENGTH_LEN]
            .copy_from_slice(&seg_len.to_be_bytes());

        buf[HEADER_LEN + CLUT_ID_OFFSET] = self.clut_id;
        buf[HEADER_LEN + CLUT_VERSION_OFFSET] = (broadcast_core::len::fit_mask(
            self.clut_version_number,
            CLUT_VERSION_RFU_MASK,
            "clut_version_number",
        )? << CLUT_VERSION_SHIFT)
            | broadcast_core::len::fit_mask(self.reserved, CLUT_VERSION_RFU_MASK, "reserved")?;
        self.clut_parameters.serialize_into(
            &mut buf[HEADER_LEN + CLUT_PARAMETERS_OFFSET
                ..HEADER_LEN + CLUT_PARAMETERS_OFFSET + CLUT_PARAMETERS_LEN],
        )?;

        let entry_len = AlternativeClutEntry::serialized_len(self.clut_parameters.output_bit_depth);
        for (i, entry) in self.entries.iter().enumerate() {
            let off = HEADER_LEN + FIXED_LEN + i * entry_len;
            entry.serialize_into(
                &mut buf[off..off + entry_len],
                self.clut_parameters.output_bit_depth,
            )?;
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_core::{Parse, Serialize};

    #[test]
    fn round_trip_8bit() {
        let bytes = [
            0x0F, 0x16, 0x00, 0x01, 0x00, 0x08, 0x03, 0x10, 0x00, 0x00, 0x80, 0x80, 0x80, 0x80,
        ];
        let seg = AlternativeClutSegment::parse(&bytes).unwrap();
        assert_eq!(seg.clut_id, 3);
        assert_eq!(seg.entries.len(), 1);
        assert_eq!(seg.entries[0].luma_value, 128);
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.entries[0].luma_value = 200;
        let out2 = seg2.try_to_bytes().unwrap();
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
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.clut_id = 7;
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        let reparse = AlternativeClutSegment::parse(&out2).unwrap();
        assert_eq!(reparse.clut_id, 7);
    }

    /// DS-W2 (#1108): sync_byte wasn't checked by any typed segment parser.
    #[test]
    fn rejects_bad_sync_byte() {
        let bytes = [
            0x00, 0x16, 0x00, 0x01, 0x00, 0x08, 0x03, 0x10, 0x00, 0x00, 0x80, 0x80, 0x80, 0x80,
        ];
        assert!(matches!(
            AlternativeClutSegment::parse(&bytes),
            Err(Error::InvalidSync(broadcast_core::error::InvalidSync {
                found: 0x00,
                ..
            }))
        ));
    }

    /// DS-W6 (#1108): Table 31's entry loop has no stuffing provision, so a
    /// trailing remainder that isn't a whole entry is now rejected, not
    /// silently dropped by integer-dividing `entry_data.len() / entry_len`.
    #[test]
    fn rejects_trailing_partial_entry() {
        let bytes = [
            0x0F, 0x16, 0x00, 0x01, 0x00, 0x0A, 0x03, 0x10, 0x00, 0x00, 0x80, 0x80, 0x80, 0x80,
            0xAA, 0xBB,
        ];
        assert!(matches!(
            AlternativeClutSegment::parse(&bytes),
            Err(Error::TrailingEntryBytes {
                entry_len: 4,
                extra: 2,
                ..
            })
        ));
    }
}
