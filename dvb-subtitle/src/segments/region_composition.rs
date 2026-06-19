//! Region Composition Segment — ETSI EN 300 743 §7.2.3, Table 11 (segment_type 0x11).

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

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
            Self::Reserved(v) => v & 0x07,
        }
    }
}

dvb_common::impl_spec_display!(RegionLevelOfCompatibility, Reserved);

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
            Self::Reserved(v) => v & 0x07,
        }
    }
}

dvb_common::impl_spec_display!(RegionDepth, Reserved);

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
            Self::Reserved(v) => v & 0x03,
        }
    }
}

dvb_common::impl_spec_display!(ObjectType, Reserved);

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
            Self::Reserved(v) => v & 0x03,
        }
    }
}

dvb_common::impl_spec_display!(ObjectProviderFlag, Reserved);

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

    fn serialize_into(&self, buf: &mut [u8]) {
        buf[0..2].copy_from_slice(&self.object_id.to_be_bytes());
        let hpos = self.object_horizontal_position;
        let vpos = self.object_vertical_position;
        buf[2] = (self.object_type.to_bits() << 6)
            | (self.object_provider_flag.to_bits() << 4)
            | ((hpos >> 8) as u8 & 0x0F);
        buf[3] = hpos as u8;
        buf[4] = ((vpos >> 8) as u8 & 0x0F) << 4; // upper nibble of vpos, lower nibble reserved=0
        buf[5] = vpos as u8;
        if let (Some(fg), Some(bg)) = (self.foreground_pixel_code, self.background_pixel_code) {
            buf[6] = fg;
            buf[7] = bg;
        }
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
    if bytes.len() < OBJECT_ENTRY_BASE_LEN {
        return Err(Error::BufferTooShort {
            need: OBJECT_ENTRY_BASE_LEN,
            have: bytes.len(),
            what: "region_object_entry",
        });
    }
    let object_id = u16::from_be_bytes([bytes[0], bytes[1]]);
    let obj_type_val = (bytes[2] >> 6) & 0x03;
    let obj_provider_val = (bytes[2] >> 4) & 0x03;
    let obj_hpos = ((u16::from(bytes[2]) & 0x0F) << 8) | u16::from(bytes[3]);
    let reserved = (bytes[4] >> 4) & 0x0F;
    // reserved bits tolerated per §7.2.0.2 forward compatibility
    let _ = reserved;
    let obj_vpos = ((u16::from(bytes[4]) & 0x0F) << 8) | u16::from(bytes[5]);

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

    let has_extra = obj_type_val == 0x01 || obj_type_val == 0x02;
    if has_extra {
        if bytes.len() < OBJECT_ENTRY_BASE_LEN + OBJECT_EXTRA_LEN {
            return Err(Error::BufferTooShort {
                need: OBJECT_ENTRY_BASE_LEN + OBJECT_EXTRA_LEN,
                have: bytes.len(),
                what: "region_object_entry extra",
            });
        }
        Ok((
            RegionObjectEntry {
                object_id,
                object_type: obj_type,
                object_provider_flag: obj_provider,
                object_horizontal_position: obj_hpos,
                object_vertical_position: obj_vpos,
                foreground_pixel_code: Some(bytes[6]),
                background_pixel_code: Some(bytes[7]),
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
        if bytes.len() < HEADER_LEN + FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + FIXED_LEN,
                have: bytes.len(),
                what: "region_composition_segment",
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
                what: "region_composition_segment data",
            });
        }
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        if body.len() < FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN,
                have: body.len(),
                what: "region_composition_segment body",
            });
        }
        let region_id = body[0];
        let region_version_number = body[1] >> 4;
        let region_fill_flag = (body[1] & 0x08) != 0;
        let reserved_byte1 = body[1] & 0x07;
        let region_width = u16::from_be_bytes([body[2], body[3]]);
        let region_height = u16::from_be_bytes([body[4], body[5]]);
        let rloc_val = (body[6] >> 5) & 0x07;
        let region_level_of_compatibility = match rloc_val {
            0x00 => RegionLevelOfCompatibility::Reserved0,
            0x01 => RegionLevelOfCompatibility::Clut2Bit,
            0x02 => RegionLevelOfCompatibility::Clut4Bit,
            0x03 => RegionLevelOfCompatibility::Clut8Bit,
            v => RegionLevelOfCompatibility::Reserved(v),
        };
        let rd_val = (body[6] >> 2) & 0x07;
        let region_depth = match rd_val {
            0x00 => RegionDepth::Reserved0,
            0x01 => RegionDepth::Depth2Bit,
            0x02 => RegionDepth::Depth4Bit,
            0x03 => RegionDepth::Depth8Bit,
            v => RegionDepth::Reserved(v),
        };
        let reserved_byte6 = body[6] & 0x03;
        let clut_id = body[7];
        let region_8bit_pixel_code = body[8];
        let region_4bit_pixel_code = body[9] >> 4;
        let region_2bit_pixel_code = (body[9] >> 2) & 0x03;
        let reserved_byte9 = body[9] & 0x03;

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
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "region_composition_segment serialize",
            });
        }
        buf[0] = 0x0F;
        buf[1] = SEGMENT_TYPE;
        buf[2..4].copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = (len - HEADER_LEN) as u16;
        buf[4..6].copy_from_slice(&seg_len.to_be_bytes());

        buf[6] = self.region_id;
        buf[7] = (self.region_version_number << 4)
            | (u8::from(self.region_fill_flag) << 3)
            | (self.reserved_byte1 & 0x07);
        buf[8..10].copy_from_slice(&self.region_width.to_be_bytes());
        buf[10..12].copy_from_slice(&self.region_height.to_be_bytes());
        buf[12] = (self.region_level_of_compatibility.to_bits() << 5)
            | (self.region_depth.to_bits() << 2)
            | (self.reserved_byte6 & 0x03);
        buf[13] = self.clut_id;
        buf[14] = self.region_8bit_pixel_code;
        buf[15] = (self.region_4bit_pixel_code << 4)
            | (self.region_2bit_pixel_code << 2)
            | (self.reserved_byte9 & 0x03);

        let mut off = HEADER_LEN + FIXED_LEN;
        for obj in &self.objects {
            obj.serialize_into(&mut buf[off..]);
            off += obj.serialized_len();
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::{Parse, Serialize};

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
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test: mutate a field and re-parse
        let mut seg2 = seg.clone();
        seg2.region_width = 1280;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = RegionCompositionSegment::parse(&out2).unwrap();
        assert_eq!(reparse.region_width, 1280);
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
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.objects[0].foreground_pixel_code = Some(0xCC);
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = RegionCompositionSegment::parse(&out2).unwrap();
        assert_eq!(reparse.objects[0].foreground_pixel_code, Some(0xCC));
    }
}
