//! Region Composition Segment — ETSI EN 300 743 §7.2.3, Table 11 (segment_type 0x11).

use crate::error::{Error, Result};
use crate::segments::{
    PAGE_ID_LEN, PAGE_ID_OFFSET, SEGMENT_LENGTH_LEN, SEGMENT_LENGTH_OFFSET, SEGMENT_TYPE_OFFSET,
    be_u16,
};
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

/// The region_composition_segment segment_type.
pub const SEGMENT_TYPE: u8 = 0x11;
/// Header: sync_byte(1) + segment_type(1) + page_id(2) + segment_length(2) = 6 bytes.
pub const HEADER_LEN: usize = 6;
/// Fixed body: 10 bytes.
pub const FIXED_LEN: usize = 10;
/// Each object entry base: object_id(2) + type(2b)+provider(2b)+hpos(12b)+reserved(4b)+vpos(12b) = 6 bytes.
pub const OBJECT_ENTRY_BASE_LEN: usize = 6;
/// Extra bytes for character/composite objects: foreground(1) + background(1) = 2 bytes.
pub const OBJECT_EXTRA_LEN: usize = 2;
/// Top 4 bits of the 12-bit object horizontal/vertical position, held in the
/// low nibble of the third/fifth entry byte (EN 300 743 Table 11).
const POS_HIGH_NIBBLE_MASK: u8 = 0x0F;

/// Offset of `object_id` within an object entry.
const OBJECT_ID_OFFSET: usize = 0;
/// Offset of the type/provider/position byte within an object entry.
const OBJECT_FLAGS_OFFSET: usize = 2;
/// Offset of the reserved nibble byte preceding `object_vertical_position`.
const OBJECT_VPOS_OFFSET: usize = 4;
/// Shift placing `object_type` at the top of its byte.
const OBJECT_TYPE_SHIFT: u32 = 6;
/// Mask of `object_type`.
const OBJECT_TYPE_MASK: u8 = 0x03;
/// Shift placing `object_provider_flag`.
const OBJECT_PROVIDER_SHIFT: u32 = 4;
/// Mask of `object_provider_flag`.
const OBJECT_PROVIDER_MASK: u8 = 0x03;
/// Shift placing the high bits of a 12-bit position.
const POS_HIGH_SHIFT: u32 = 8;
/// Bit width of `object_horizontal_position` / `object_vertical_position`.
const OBJECT_POS_BITS: u32 = 12;
/// Shift selecting the reserved nibble of the `vpos` byte.
const OBJECT_RESERVED_SHIFT: u32 = 4;
/// Offsets of the foreground/background pixel codes.
const OBJECT_FOREGROUND_OFFSET: usize = 6;
const OBJECT_BACKGROUND_OFFSET: usize = 7;

/// Offset of `region_id` within the segment body.
const REGION_ID_OFFSET: usize = 0;
/// Offset of the version/fill/reserved byte within the segment body.
const REGION_FLAGS_OFFSET: usize = 1;
/// Offset of `region_width` within the segment body.
const REGION_WIDTH_OFFSET: usize = 2;
/// Offset of `region_height` within the segment body.
const REGION_HEIGHT_OFFSET: usize = 4;
/// Offset of the level/depth/reserved byte within the segment body.
const REGION_DEPTH_OFFSET: usize = 6;
/// Offset of `clut_id` within the segment body.
const CLUT_ID_OFFSET: usize = 7;
/// Offset of `region_8bit_pixel_code` within the segment body.
const PIXEL_CODE_8BIT_OFFSET: usize = 8;
/// Offset of the 4-bit/2-bit pixel-code byte within the segment body.
const PIXEL_CODE_2_4BIT_OFFSET: usize = 9;

/// Shift placing `region_version_number`.
const REGION_VERSION_SHIFT: u32 = 4;
/// Mask of `region_version_number`.
const REGION_VERSION_MASK: u8 = 0x0F;
/// Mask of `region_fill_flag`.
const REGION_FILL_FLAG_MASK: u8 = 0x08;
/// Shift placing `region_fill_flag`.
const REGION_FILL_FLAG_SHIFT: u32 = 3;
/// Mask of the reserved bits in the flags byte.
const REGION_RFU_MASK: u8 = 0x07;
/// Shift placing `region_level_of_compatibility`.
const RLOC_SHIFT: u32 = 5;
/// Mask of `region_level_of_compatibility`.
const RLOC_MASK: u8 = 0x07;
/// Shift placing `region_depth`.
const REGION_DEPTH_SHIFT: u32 = 2;
/// Mask of `region_depth`.
const REGION_DEPTH_MASK: u8 = 0x07;
/// Mask of the reserved bits in the depth byte.
const DEPTH_RFU_MASK: u8 = 0x03;
/// Shift placing `region_4bit_pixel_code`.
const PIXEL_CODE_4BIT_SHIFT: u32 = 4;
/// Mask of `region_4bit_pixel_code`.
const PIXEL_CODE_4BIT_MASK: u8 = 0x0F;
/// Shift placing `region_2bit_pixel_code`.
const PIXEL_CODE_2BIT_SHIFT: u32 = 2;
/// Mask of `region_2bit_pixel_code`.
const PIXEL_CODE_2BIT_MASK: u8 = 0x03;

/// Region level of compatibility as defined in Table 12.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum RegionLevelOfCompatibility {
    /// Reserved.
    Reserved0 = 0x00,
    /// 2-bit/entry CLUT required.
    Clut2Bit = 0x01,
    /// 4-bit/entry CLUT required.
    Clut4Bit = 0x02,
    /// 8-bit/entry CLUT required.
    Clut8Bit = 0x03,
    /// Reserved range.
    Reserved(u8),
}

impl RegionLevelOfCompatibility {
    /// Human-readable name for this compatibility level.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved0 => "reserved",
            Self::Clut2Bit => "2-bit CLUT",
            Self::Clut4Bit => "4-bit CLUT",
            Self::Clut8Bit => "8-bit CLUT",
            Self::Reserved(_) => "reserved",
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::Reserved0 => 0x00,
            Self::Clut2Bit => 0x01,
            Self::Clut4Bit => 0x02,
            Self::Clut8Bit => 0x03,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_core::impl_spec_display!(RegionLevelOfCompatibility, Reserved);

/// Intended region pixel depth as defined in Table 13.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum RegionDepth {
    /// Reserved.
    Reserved0 = 0x00,
    /// 2-bit.
    Depth2Bit = 0x01,
    /// 4-bit.
    Depth4Bit = 0x02,
    /// 8-bit.
    Depth8Bit = 0x03,
    /// Reserved range.
    Reserved(u8),
}

impl RegionDepth {
    /// Human-readable name for this pixel depth.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved0 => "reserved",
            Self::Depth2Bit => "2-bit",
            Self::Depth4Bit => "4-bit",
            Self::Depth8Bit => "8-bit",
            Self::Reserved(_) => "reserved",
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::Reserved0 => 0x00,
            Self::Depth2Bit => 0x01,
            Self::Depth4Bit => 0x02,
            Self::Depth8Bit => 0x03,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_core::impl_spec_display!(RegionDepth, Reserved);

/// Object type as defined in Table 14.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum ObjectType {
    /// Basic object, bitmap.
    BasicBitmap = 0x00,
    /// Basic object, character.
    BasicCharacter = 0x01,
    /// Composite object, string of characters.
    CompositeString = 0x02,
    /// Reserved.
    Reserved(u8),
}

impl ObjectType {
    /// Human-readable name for this object type.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::BasicBitmap => "basic_bitmap",
            Self::BasicCharacter => "basic_character",
            Self::CompositeString => "composite_string",
            Self::Reserved(_) => "reserved",
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::BasicBitmap => 0x00,
            Self::BasicCharacter => 0x01,
            Self::CompositeString => 0x02,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_core::impl_spec_display!(ObjectType, Reserved);

/// Object provider flag as defined in Table 15.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum ObjectProviderFlag {
    /// Provided in the subtitling stream.
    InStream = 0x00,
    /// Provided by a ROM in the IRD.
    InRom = 0x01,
    /// Reserved.
    Reserved(u8),
}

impl ObjectProviderFlag {
    /// Human-readable name for this provision method.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::InStream => "in_stream",
            Self::InRom => "in_rom",
            Self::Reserved(_) => "reserved",
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::InStream => 0x00,
            Self::InRom => 0x01,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_core::impl_spec_display!(ObjectProviderFlag, Reserved);

/// An object entry within a region composition segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RegionObjectEntry {
    /// Object identifier.
    pub object_id: u16,
    /// Object type.
    pub object_type: ObjectType,
    /// Object provider flag.
    pub object_provider_flag: ObjectProviderFlag,
    /// Horizontal position relative to region.
    pub object_horizontal_position: u16,
    /// Vertical position relative to region.
    pub object_vertical_position: u16,
    /// Foreground pixel code (only for character/composite objects).
    pub foreground_pixel_code: Option<u8>,
    /// Background pixel code (only for character/composite objects).
    pub background_pixel_code: Option<u8>,
}

impl RegionObjectEntry {
    fn serialized_len(&self) -> usize {
        if self.foreground_pixel_code.is_some() {
            OBJECT_ENTRY_BASE_LEN + OBJECT_EXTRA_LEN
        } else {
            OBJECT_ENTRY_BASE_LEN
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<()> {
        buf[..OBJECT_FLAGS_OFFSET].copy_from_slice(&self.object_id.to_be_bytes());
        let hpos = self.object_horizontal_position;
        let vpos = self.object_vertical_position;
        // EN 300 743 Table 11 (private/specs-verified/dvb/dvb-subtitle/tables/en_300_743/11-region-composition-segment.md:29-33):
        // both object_horizontal_position and object_vertical_position are
        // 12-bit uimsbf fields; range-check before narrowing so a value that
        // does not fit is rejected instead of silently masked (#1129/#1044).
        broadcast_core::len::fit_bits(
            u64::from(hpos),
            OBJECT_POS_BITS,
            "object_horizontal_position",
        )?;
        broadcast_core::len::fit_bits(
            u64::from(vpos),
            OBJECT_POS_BITS,
            "object_vertical_position",
        )?;
        buf[OBJECT_FLAGS_OFFSET] = (self.object_type.to_bits() << OBJECT_TYPE_SHIFT)
            | (self.object_provider_flag.to_bits() << OBJECT_PROVIDER_SHIFT)
            | ((hpos >> POS_HIGH_SHIFT) as u8 & POS_HIGH_NIBBLE_MASK);
        buf[OBJECT_FLAGS_OFFSET + 1] = hpos as u8;
        // reserved(4) then the top 4 bits of vpos, i.e. reserved is the HIGH
        // nibble and vpos's top bits are the LOW nibble — the mirror of
        // `parse`'s `(bytes[4] & 0x0F) << 8`.
        buf[OBJECT_VPOS_OFFSET] = (vpos >> POS_HIGH_SHIFT) as u8 & POS_HIGH_NIBBLE_MASK;
        buf[OBJECT_VPOS_OFFSET + 1] = vpos as u8;
        if let (Some(fg), Some(bg)) = (self.foreground_pixel_code, self.background_pixel_code) {
            buf[OBJECT_FOREGROUND_OFFSET] = fg;
            buf[OBJECT_BACKGROUND_OFFSET] = bg;
        }
        Ok(())
    }
}

/// Region Composition Segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RegionCompositionSegment {
    /// The page_id (from generic segment header).
    pub page_id: u16,
    /// Region identifier.
    pub region_id: u8,
    /// Region version number (modulo 16).
    pub region_version_number: u8,
    /// Fill flag.
    pub region_fill_flag: bool,
    /// Reserved bits in body byte 1 (bits `[2:0]`).
    pub reserved_byte1: u8,
    /// Region width in pixels.
    pub region_width: u16,
    /// Region height in pixels.
    pub region_height: u16,
    /// Minimum CLUT type required.
    pub region_level_of_compatibility: RegionLevelOfCompatibility,
    /// Intended pixel depth.
    pub region_depth: RegionDepth,
    /// Reserved bits in body byte 6 (bits `[1:0]`).
    pub reserved_byte6: u8,
    /// CLUT family identifier.
    pub clut_id: u8,
    /// Background colour for 8-bit CLUT.
    pub region_8bit_pixel_code: u8,
    /// Background colour for 4-bit CLUT.
    pub region_4bit_pixel_code: u8,
    /// Background colour for 2-bit CLUT.
    pub region_2bit_pixel_code: u8,
    /// Reserved bits in body byte 9 (bits `[1:0]`).
    pub reserved_byte9: u8,
    /// Object entries.
    pub objects: alloc::vec::Vec<RegionObjectEntry>,
}

fn parse_object_entry(bytes: &[u8]) -> Result<(RegionObjectEntry, usize)> {
    need_in(bytes, OBJECT_ENTRY_BASE_LEN, "region_object_entry")?;
    let object_id = be_u16(bytes, OBJECT_ID_OFFSET);
    let obj_type_val = (bytes[OBJECT_FLAGS_OFFSET] >> OBJECT_TYPE_SHIFT) & OBJECT_TYPE_MASK;
    let obj_provider_val =
        (bytes[OBJECT_FLAGS_OFFSET] >> OBJECT_PROVIDER_SHIFT) & OBJECT_PROVIDER_MASK;
    let obj_hpos = ((u16::from(bytes[OBJECT_FLAGS_OFFSET]) & u16::from(POS_HIGH_NIBBLE_MASK))
        << POS_HIGH_SHIFT)
        | u16::from(bytes[OBJECT_FLAGS_OFFSET + 1]);
    let reserved = (bytes[OBJECT_VPOS_OFFSET] >> OBJECT_RESERVED_SHIFT) & POS_HIGH_NIBBLE_MASK;
    // reserved bits tolerated per §7.2.0.2 forward compatibility
    let _ = reserved;
    let obj_vpos = ((u16::from(bytes[OBJECT_VPOS_OFFSET]) & u16::from(POS_HIGH_NIBBLE_MASK))
        << POS_HIGH_SHIFT)
        | u16::from(bytes[OBJECT_VPOS_OFFSET + 1]);

    let obj_type = match obj_type_val {
        0x00 => ObjectType::BasicBitmap,
        0x01 => ObjectType::BasicCharacter,
        0x02 => ObjectType::CompositeString,
        v => ObjectType::Reserved(v),
    };
    let obj_provider = match obj_provider_val {
        0x00 => ObjectProviderFlag::InStream,
        0x01 => ObjectProviderFlag::InRom,
        v => ObjectProviderFlag::Reserved(v),
    };

    let has_extra = obj_type_val == ObjectType::BasicCharacter.to_bits()
        || obj_type_val == ObjectType::CompositeString.to_bits();
    if has_extra {
        need_in(
            bytes,
            OBJECT_ENTRY_BASE_LEN + OBJECT_EXTRA_LEN,
            "region_object_entry extra",
        )?;
        Ok((
            RegionObjectEntry {
                object_id,
                object_type: obj_type,
                object_provider_flag: obj_provider,
                object_horizontal_position: obj_hpos,
                object_vertical_position: obj_vpos,
                foreground_pixel_code: Some(bytes[OBJECT_FOREGROUND_OFFSET]),
                background_pixel_code: Some(bytes[OBJECT_BACKGROUND_OFFSET]),
            },
            OBJECT_ENTRY_BASE_LEN + OBJECT_EXTRA_LEN,
        ))
    } else {
        Ok((
            RegionObjectEntry {
                object_id,
                object_type: obj_type,
                object_provider_flag: obj_provider,
                object_horizontal_position: obj_hpos,
                object_vertical_position: obj_vpos,
                foreground_pixel_code: None,
                background_pixel_code: None,
            },
            OBJECT_ENTRY_BASE_LEN,
        ))
    }
}

impl<'a> Parse<'a> for RegionCompositionSegment {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, HEADER_LEN + FIXED_LEN, "region_composition_segment")?;
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
        need_in(bytes, total, "region_composition_segment data")?;
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        need_in(body, FIXED_LEN, "region_composition_segment body")?;
        let region_id = body[REGION_ID_OFFSET];
        let region_version_number = body[REGION_FLAGS_OFFSET] >> REGION_VERSION_SHIFT;
        let region_fill_flag = (body[REGION_FLAGS_OFFSET] & REGION_FILL_FLAG_MASK) != 0;
        let reserved_byte1 = body[REGION_FLAGS_OFFSET] & REGION_RFU_MASK;
        let region_width = be_u16(body, REGION_WIDTH_OFFSET);
        let region_height = be_u16(body, REGION_HEIGHT_OFFSET);
        let rloc_val = (body[REGION_DEPTH_OFFSET] >> RLOC_SHIFT) & RLOC_MASK;
        let region_level_of_compatibility = match rloc_val {
            0x00 => RegionLevelOfCompatibility::Reserved0,
            0x01 => RegionLevelOfCompatibility::Clut2Bit,
            0x02 => RegionLevelOfCompatibility::Clut4Bit,
            0x03 => RegionLevelOfCompatibility::Clut8Bit,
            v => RegionLevelOfCompatibility::Reserved(v),
        };
        let rd_val = (body[REGION_DEPTH_OFFSET] >> REGION_DEPTH_SHIFT) & REGION_DEPTH_MASK;
        let region_depth = match rd_val {
            0x00 => RegionDepth::Reserved0,
            0x01 => RegionDepth::Depth2Bit,
            0x02 => RegionDepth::Depth4Bit,
            0x03 => RegionDepth::Depth8Bit,
            v => RegionDepth::Reserved(v),
        };
        let reserved_byte6 = body[REGION_DEPTH_OFFSET] & DEPTH_RFU_MASK;
        let clut_id = body[CLUT_ID_OFFSET];
        let region_8bit_pixel_code = body[PIXEL_CODE_8BIT_OFFSET];
        let region_4bit_pixel_code = body[PIXEL_CODE_2_4BIT_OFFSET] >> PIXEL_CODE_4BIT_SHIFT;
        let region_2bit_pixel_code =
            (body[PIXEL_CODE_2_4BIT_OFFSET] >> PIXEL_CODE_2BIT_SHIFT) & PIXEL_CODE_2BIT_MASK;
        let reserved_byte9 = body[PIXEL_CODE_2_4BIT_OFFSET] & DEPTH_RFU_MASK;

        let obj_data = &body[FIXED_LEN..];
        let mut objects = alloc::vec::Vec::new();
        let mut pos: usize = 0;
        while pos < obj_data.len() {
            let (entry, entry_len) = parse_object_entry(&obj_data[pos..])?;
            objects.push(entry);
            pos += entry_len;
        }

        Ok(RegionCompositionSegment {
            page_id,
            region_id,
            region_version_number,
            region_fill_flag,
            reserved_byte1,
            region_width,
            region_height,
            region_level_of_compatibility,
            region_depth,
            reserved_byte6,
            clut_id,
            region_8bit_pixel_code,
            region_4bit_pixel_code,
            region_2bit_pixel_code,
            reserved_byte9,
            objects,
        })
    }
}

impl Serialize for RegionCompositionSegment {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + FIXED_LEN
            + self
                .objects
                .iter()
                .map(|o| o.serialized_len())
                .sum::<usize>()
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

        buf[HEADER_LEN + REGION_ID_OFFSET] = self.region_id;
        buf[HEADER_LEN + REGION_FLAGS_OFFSET] = (broadcast_core::len::fit_mask(
            self.region_version_number,
            REGION_VERSION_MASK,
            "region_version_number",
        )? << REGION_VERSION_SHIFT)
            | (u8::from(self.region_fill_flag) << REGION_FILL_FLAG_SHIFT)
            | broadcast_core::len::fit_mask(
                self.reserved_byte1,
                REGION_RFU_MASK,
                "reserved_byte1",
            )?;
        buf[HEADER_LEN + REGION_WIDTH_OFFSET..HEADER_LEN + REGION_HEIGHT_OFFSET]
            .copy_from_slice(&self.region_width.to_be_bytes());
        buf[HEADER_LEN + REGION_HEIGHT_OFFSET..HEADER_LEN + REGION_DEPTH_OFFSET]
            .copy_from_slice(&self.region_height.to_be_bytes());
        buf[HEADER_LEN + REGION_DEPTH_OFFSET] = (broadcast_core::len::fit_mask(
            self.region_level_of_compatibility.to_bits(),
            RLOC_MASK,
            "region_level_of_compatibility",
        )? << RLOC_SHIFT)
            | (broadcast_core::len::fit_mask(
                self.region_depth.to_bits(),
                REGION_DEPTH_MASK,
                "region_depth",
            )? << REGION_DEPTH_SHIFT)
            | broadcast_core::len::fit_mask(self.reserved_byte6, DEPTH_RFU_MASK, "reserved_byte6")?;
        buf[HEADER_LEN + CLUT_ID_OFFSET] = self.clut_id;
        buf[HEADER_LEN + PIXEL_CODE_8BIT_OFFSET] = self.region_8bit_pixel_code;
        buf[HEADER_LEN + PIXEL_CODE_2_4BIT_OFFSET] = (broadcast_core::len::fit_mask(
            self.region_4bit_pixel_code,
            PIXEL_CODE_4BIT_MASK,
            "region_4bit_pixel_code",
        )? << PIXEL_CODE_4BIT_SHIFT)
            | (broadcast_core::len::fit_mask(
                self.region_2bit_pixel_code,
                PIXEL_CODE_2BIT_MASK,
                "region_2bit_pixel_code",
            )? << PIXEL_CODE_2BIT_SHIFT)
            | broadcast_core::len::fit_mask(self.reserved_byte9, DEPTH_RFU_MASK, "reserved_byte9")?;

        let mut off = HEADER_LEN + FIXED_LEN;
        for obj in &self.objects {
            obj.serialize_into(&mut buf[off..])?;
            off += obj.serialized_len();
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_core::{Parse, Serialize};

    #[test]
    fn round_trip_bitmap_objects() {
        let bytes = [
            0x0F, 0x11, 0x00, 0x01, 0x00, 0x16, 0x01, 0x88, 0x02, 0xCF, 0x00, 0x8F, 0x18, 0x03,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x0A, 0x00, 0x0A, 0x00, 0x02, 0x10, 0x14, 0x00,
            0x14, // obj 2: hpos=20, vpos=20, vpos upper nibble=0
        ];
        let seg = RegionCompositionSegment::parse(&bytes).unwrap();
        assert_eq!(seg.region_id, 1);
        assert!(seg.region_fill_flag);
        assert_eq!(seg.region_width, 719);
        assert_eq!(seg.objects.len(), 2);
        assert_eq!(seg.objects[0].object_id, 1);
        assert_eq!(seg.objects[0].object_type, ObjectType::BasicBitmap);
        assert_eq!(seg.objects[0].object_horizontal_position, 10);
        assert_eq!(seg.objects[1].object_id, 2);
        assert_eq!(
            seg.objects[1].object_provider_flag,
            ObjectProviderFlag::InRom
        );
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test: mutate a field and re-parse
        let mut seg2 = seg.clone();
        seg2.region_width = 1280;
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        let reparse = RegionCompositionSegment::parse(&out2).unwrap();
        assert_eq!(reparse.region_width, 1280);
    }

    /// DS-W2 (#1108): sync_byte wasn't checked by any typed segment parser.
    #[test]
    fn rejects_bad_sync_byte() {
        let bytes = [
            0x00, 0x11, 0x00, 0x01, 0x00, 0x16, 0x01, 0x88, 0x02, 0xCF, 0x00, 0x8F, 0x18, 0x03,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x0A, 0x00, 0x0A, 0x00, 0x02, 0x10, 0x14, 0x00, 0x14,
        ];
        assert!(matches!(
            RegionCompositionSegment::parse(&bytes),
            Err(Error::InvalidSync(broadcast_core::error::InvalidSync {
                found: 0x00,
                ..
            }))
        ));
    }

    #[test]
    fn round_trip_character_object() {
        let bytes = [
            0x0F, 0x11, 0x00, 0x01, 0x00, 0x12, 0x01, 0x80, 0x02, 0xCF, 0x00, 0x8F, 0x18, 0x03,
            0x00, 0x00, 0x00, 0x01, 0x40, 0x0A, 0x00, 0x1E, 0xAA, 0xBB,
        ];
        let seg = RegionCompositionSegment::parse(&bytes).unwrap();
        assert_eq!(seg.objects.len(), 1);
        assert_eq!(seg.objects[0].object_type, ObjectType::BasicCharacter);
        assert_eq!(seg.objects[0].foreground_pixel_code, Some(0xAA));
        assert_eq!(seg.objects[0].background_pixel_code, Some(0xBB));
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.objects[0].foreground_pixel_code = Some(0xCC);
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        let reparse = RegionCompositionSegment::parse(&out2).unwrap();
        assert_eq!(reparse.objects[0].foreground_pixel_code, Some(0xCC));
    }

    /// Issue #1044: `object_vertical_position >= 0x100` (>= 256) must
    /// round-trip. EN 300 743 Table 11 (`private/specs-verified/dvb/dvb-subtitle/tables/en_300_743/11-region-composition-segment.md:32-33`):
    /// byte 4 of the object entry is `reserved(4)` then the *top 4 bits* of
    /// the 12-bit `object_vertical_position` — the same split `parse` already
    /// uses (`(bytes[4] & 0x0F) << 8 | bytes[5]`). Every existing round-trip
    /// fixture keeps `vpos < 256` (a single byte), so this is the first test
    /// to exercise the top nibble.
    #[test]
    fn round_trip_object_vertical_position_above_one_byte() {
        let mut seg = RegionCompositionSegment::parse(&[
            0x0F, 0x11, 0x00, 0x01, 0x00, 0x16, 0x01, 0x88, 0x02, 0xCF, 0x00, 0x8F, 0x18, 0x03,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x0A, 0x00, 0x0A, 0x00, 0x02, 0x10, 0x14, 0x00, 0x14,
        ])
        .unwrap();
        // 900 (0x384): the bottom-of-a-1080-line-frame subtitle position from
        // the audit's failure scenario.
        seg.objects[1].object_vertical_position = 0x384;

        let out = seg.try_to_bytes().unwrap();
        // object index 1 is the last (and, per the fixture, fg/bg-less 6-byte)
        // entry in the loop, so its `object_vertical_position` bytes are the
        // final 2 bytes of the whole serialized segment. They must be
        // `reserved(0) | vpos[11:8]` = 0x03 (not 0x30 — that would put the
        // position's top nibble into the reserved high nibble, the exact
        // bug) then `vpos[7:0]` = 0x84.
        let n = out.len();
        assert_eq!(
            out[n - 2],
            0x03,
            "vpos top nibble must land in the low nibble of byte 4, not the reserved high nibble"
        );
        assert_eq!(out[n - 1], 0x84);

        let reparsed = RegionCompositionSegment::parse(&out).unwrap();
        assert_eq!(
            reparsed.objects[1].object_vertical_position, 0x384,
            "byte-exact round-trip of a vertical position >= 256"
        );
    }

    /// Issue #1044/#1129: `object_vertical_position`/`object_horizontal_position`
    /// are 12-bit fields (EN 300 743 Table 11); a value over `0x0FFF` must be
    /// rejected, not silently masked to a wrong (wrapped) position.
    #[test]
    fn oversized_object_position_is_rejected_not_masked() {
        let mut seg = RegionCompositionSegment::parse(&[
            0x0F, 0x11, 0x00, 0x01, 0x00, 0x16, 0x01, 0x88, 0x02, 0xCF, 0x00, 0x8F, 0x18, 0x03,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x0A, 0x00, 0x0A, 0x00, 0x02, 0x10, 0x14, 0x00, 0x14,
        ])
        .unwrap();
        let mut buf = alloc::vec![0u8; 64];

        seg.objects[1].object_vertical_position = 0x1000; // 13 bits, one over max
        assert!(matches!(
            seg.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));

        seg.objects[1].object_vertical_position = 0x0FFF; // the 12-bit maximum
        seg.objects[1].object_horizontal_position = 0x1000;
        assert!(matches!(
            seg.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));

        seg.objects[1].object_horizontal_position = 0x0FFF; // both at max: OK
        assert!(seg.serialize_into(&mut buf).is_ok());
    }
}
