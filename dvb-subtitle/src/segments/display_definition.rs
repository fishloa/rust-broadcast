//! Display Definition Segment — ETSI EN 300 743 §7.2.1, Table 8 (segment_type 0x14).
//!
//! Defines the display resolution and optional display window for a subtitle service.

use crate::error::{Error, Result};
use crate::segments::{
    PAGE_ID_LEN, PAGE_ID_OFFSET, SEGMENT_LENGTH_LEN, SEGMENT_LENGTH_OFFSET, SEGMENT_TYPE_OFFSET,
};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// The display_definition_segment segment_type as listed in Table 7.
pub const SEGMENT_TYPE: u8 = 0x14;
/// Segment header is sync_byte(1) + segment_type(1) + page_id(2) + segment_length(2) = 6 bytes.
pub const HEADER_LEN: usize = 6;
/// Fixed segment body (after header): dds_version_number(4b)+display_window_flag(1b)+reserved(3b) + display_width(2) + display_height(2) = 5 bytes.
pub const FIXED_BODY_LEN: usize = 5;

/// Offset of the version/window-flag/reserved byte within the segment body.
const FLAGS_OFFSET: usize = 0;
/// Offset of `display_width` within the segment body.
const DISPLAY_WIDTH_OFFSET: usize = 1;
/// Offset of `display_height` within the segment body.
const DISPLAY_HEIGHT_OFFSET: usize = 3;
/// Shift placing `dds_version_number` at the top of the flags byte.
const DDS_VERSION_SHIFT: u32 = 4;
/// Mask of `dds_version_number`.
const DDS_VERSION_MASK: u8 = 0x0F;
/// Mask of the `display_window_flag`.
const DISPLAY_WINDOW_FLAG_MASK: u8 = 0x08;
/// Shift placing the `display_window_flag` below the version nibble.
const DISPLAY_WINDOW_FLAG_SHIFT: u32 = 3;
/// Mask of the reserved bits beside the window flag.
const RESERVED_MASK: u8 = 0x07;

/// Offsets of the four display-window position fields.
const WINDOW_HORIZONTAL_MIN_OFFSET: usize = 0;
const WINDOW_HORIZONTAL_MAX_OFFSET: usize = 2;
const WINDOW_VERTICAL_MIN_OFFSET: usize = 4;
const WINDOW_VERTICAL_MAX_OFFSET: usize = 6;
/// Bytes of a display-window position field.
const WINDOW_FIELD_LEN: usize = 2;
/// Optional display window fields: 4 × u16 = 8 bytes.
pub const WINDOW_LEN: usize = 8;

/// Display Definition Segment (DDS).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DisplayDefinitionSegment {
    /// The page_id (from generic segment header).
    pub page_id: u16,
    /// Version of this DDS (modulo 16).
    pub dds_version_number: u8,
    /// Whether a window is defined (`true`) or the display set fills the full resolution (`false`).
    pub display_window_flag: bool,
    /// Reserved bits in body byte 0 (bits `[2:0]`).
    pub reserved: u8,
    /// Maximum horizontal width in pixels minus 1 (0..4095).
    pub display_width: u16,
    /// Maximum vertical height in lines minus 1 (0..4095).
    pub display_height: u16,
    /// Left-most pixel of the display window (present iff `display_window_flag`).
    pub display_window_horizontal_position_minimum: Option<u16>,
    /// Right-most pixel of the display window (present iff `display_window_flag`).
    pub display_window_horizontal_position_maximum: Option<u16>,
    /// Upper-most line of the display window (present iff `display_window_flag`).
    pub display_window_vertical_position_minimum: Option<u16>,
    /// Bottom line of the display window (present iff `display_window_flag`).
    pub display_window_vertical_position_maximum: Option<u16>,
}

impl<'a> Parse<'a> for DisplayDefinitionSegment {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min = HEADER_LEN + FIXED_BODY_LEN;
        need_in(bytes, min, "display_definition_segment")?;
        if bytes[0] != crate::pes_data_field::SYNC_BYTE {
            return Err(Error::InvalidSync(broadcast_common::error::InvalidSync {
                what: "sync_byte",
                expected: u32::from(crate::pes_data_field::SYNC_BYTE),
                found: u32::from(bytes[0]),
            }));
        }
        if bytes[SEGMENT_TYPE_OFFSET] != SEGMENT_TYPE {
            return Err(Error::UnknownSegmentType(bytes[SEGMENT_TYPE_OFFSET]));
        }
        let page_id = u16::from_be_bytes([bytes[PAGE_ID_OFFSET], bytes[PAGE_ID_OFFSET + 1]]);
        let segment_length = u16::from_be_bytes([
            bytes[SEGMENT_LENGTH_OFFSET],
            bytes[SEGMENT_LENGTH_OFFSET + 1],
        ]) as usize;
        let total = HEADER_LEN + segment_length;
        need_in(bytes, total, "display_definition_segment data")?;
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        need_in(body, FIXED_BODY_LEN, "display_definition_segment body")?;
        let b0 = body[FLAGS_OFFSET];
        let dds_version_number = b0 >> DDS_VERSION_SHIFT;
        let display_window_flag = (b0 & DISPLAY_WINDOW_FLAG_MASK) != 0;
        let reserved = b0 & RESERVED_MASK;
        let display_width =
            u16::from_be_bytes([body[DISPLAY_WIDTH_OFFSET], body[DISPLAY_WIDTH_OFFSET + 1]]);
        let display_height =
            u16::from_be_bytes([body[DISPLAY_HEIGHT_OFFSET], body[DISPLAY_HEIGHT_OFFSET + 1]]);

        if display_window_flag {
            need_in(
                body,
                FIXED_BODY_LEN + WINDOW_LEN,
                "display_definition_segment window",
            )?;
            let dw = &body[FIXED_BODY_LEN..FIXED_BODY_LEN + WINDOW_LEN];
            Ok(DisplayDefinitionSegment {
                page_id,
                dds_version_number,
                display_window_flag,
                reserved,
                display_width,
                display_height,
                display_window_horizontal_position_minimum: Some(u16::from_be_bytes([
                    dw[WINDOW_HORIZONTAL_MIN_OFFSET],
                    dw[WINDOW_HORIZONTAL_MIN_OFFSET + 1],
                ])),
                display_window_horizontal_position_maximum: Some(u16::from_be_bytes([
                    dw[WINDOW_HORIZONTAL_MAX_OFFSET],
                    dw[WINDOW_HORIZONTAL_MAX_OFFSET + 1],
                ])),
                display_window_vertical_position_minimum: Some(u16::from_be_bytes([
                    dw[WINDOW_VERTICAL_MIN_OFFSET],
                    dw[WINDOW_VERTICAL_MIN_OFFSET + 1],
                ])),
                display_window_vertical_position_maximum: Some(u16::from_be_bytes([
                    dw[WINDOW_VERTICAL_MAX_OFFSET],
                    dw[WINDOW_VERTICAL_MAX_OFFSET + 1],
                ])),
            })
        } else {
            Ok(DisplayDefinitionSegment {
                page_id,
                dds_version_number,
                display_window_flag,
                reserved,
                display_width,
                display_height,
                display_window_horizontal_position_minimum: None,
                display_window_horizontal_position_maximum: None,
                display_window_vertical_position_minimum: None,
                display_window_vertical_position_maximum: None,
            })
        }
    }
}

impl Serialize for DisplayDefinitionSegment {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + FIXED_BODY_LEN
            + if self.display_window_flag {
                WINDOW_LEN
            } else {
                0
            }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        // Generic header
        buf[0] = crate::pes_data_field::SYNC_BYTE; // sync_byte
        buf[SEGMENT_TYPE_OFFSET] = SEGMENT_TYPE;
        buf[PAGE_ID_OFFSET..PAGE_ID_OFFSET + PAGE_ID_LEN]
            .copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = crate::segments::check_segment_length(len - HEADER_LEN)?;
        buf[SEGMENT_LENGTH_OFFSET..SEGMENT_LENGTH_OFFSET + SEGMENT_LENGTH_LEN]
            .copy_from_slice(&seg_len.to_be_bytes());

        // Fixed body byte 0: version(4b) | window_flag(1b) | reserved(3b)
        buf[HEADER_LEN + FLAGS_OFFSET] = (broadcast_common::len::fit_mask(
            self.dds_version_number,
            DDS_VERSION_MASK,
            "dds_version_number",
        )? << DDS_VERSION_SHIFT)
            | (u8::from(self.display_window_flag) << DISPLAY_WINDOW_FLAG_SHIFT)
            | broadcast_common::len::fit_mask(self.reserved, RESERVED_MASK, "reserved")?;
        // Fixed body bytes 1-4: display_width, display_height
        buf[HEADER_LEN + DISPLAY_WIDTH_OFFSET..HEADER_LEN + DISPLAY_HEIGHT_OFFSET]
            .copy_from_slice(&self.display_width.to_be_bytes());
        buf[HEADER_LEN + DISPLAY_HEIGHT_OFFSET..HEADER_LEN + FIXED_BODY_LEN]
            .copy_from_slice(&self.display_height.to_be_bytes());

        // Optional window
        if self.display_window_flag {
            let wmin = self.display_window_horizontal_position_minimum.unwrap_or(0);
            let wmax = self.display_window_horizontal_position_maximum.unwrap_or(0);
            let vmin = self.display_window_vertical_position_minimum.unwrap_or(0);
            let vmax = self.display_window_vertical_position_maximum.unwrap_or(0);
            let base = HEADER_LEN + FIXED_BODY_LEN;
            buf[base + WINDOW_HORIZONTAL_MIN_OFFSET..base + WINDOW_HORIZONTAL_MAX_OFFSET]
                .copy_from_slice(&wmin.to_be_bytes());
            buf[base + WINDOW_HORIZONTAL_MAX_OFFSET..base + WINDOW_VERTICAL_MIN_OFFSET]
                .copy_from_slice(&wmax.to_be_bytes());
            buf[base + WINDOW_VERTICAL_MIN_OFFSET..base + WINDOW_VERTICAL_MAX_OFFSET]
                .copy_from_slice(&vmin.to_be_bytes());
            buf[base + WINDOW_VERTICAL_MAX_OFFSET
                ..base + WINDOW_VERTICAL_MAX_OFFSET + WINDOW_FIELD_LEN]
                .copy_from_slice(&vmax.to_be_bytes());
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    #[test]
    fn round_trip_no_window() {
        let bytes = [
            0x0F, 0x14, 0x00, 0x01, 0x00, 0x05, 0x30, 0x02, 0xCF, 0x01, 0x1F,
        ];
        let seg = DisplayDefinitionSegment::parse(&bytes).unwrap();
        assert_eq!(seg.dds_version_number, 3);
        assert!(!seg.display_window_flag);
        assert_eq!(seg.display_width, 719);
        assert_eq!(seg.display_height, 287);
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test: mutation changes output
        let mut seg2 = seg.clone();
        seg2.display_width = 100;
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        let reparse = DisplayDefinitionSegment::parse(&out2).unwrap();
        assert_eq!(reparse.display_width, 100);
    }

    /// DS-W2 (#1108): sync_byte wasn't checked by any typed segment parser.
    #[test]
    fn rejects_bad_sync_byte() {
        let bytes = [
            0x00, 0x14, 0x00, 0x01, 0x00, 0x05, 0x30, 0x02, 0xCF, 0x01, 0x1F,
        ];
        assert!(matches!(
            DisplayDefinitionSegment::parse(&bytes),
            Err(Error::InvalidSync(broadcast_common::error::InvalidSync {
                found: 0x00,
                ..
            }))
        ));
    }

    #[test]
    fn round_trip_with_window() {
        let bytes = [
            0x0F, 0x14, 0x00, 0x01, 0x00, 0x0D, 0x48, 0x02, 0xCF, 0x01, 0x1F, 0x00, 0x32, 0x02,
            0x9E, 0x00, 0x14, 0x01, 0x0C,
        ];
        let seg = DisplayDefinitionSegment::parse(&bytes).unwrap();
        assert_eq!(seg.dds_version_number, 4);
        assert!(seg.display_window_flag);
        assert_eq!(seg.display_window_horizontal_position_minimum, Some(50));
        assert_eq!(seg.display_window_horizontal_position_maximum, Some(670));
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test: clearing the flag drops window
        let mut seg2 = seg.clone();
        seg2.display_window_flag = false;
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2.len(), out.len());
        let reparse = DisplayDefinitionSegment::parse(&out2).unwrap();
        assert!(!reparse.display_window_flag);
    }
}
