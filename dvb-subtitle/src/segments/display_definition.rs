//! Display Definition Segment — ETSI EN 300 743 §7.2.1, Table 8 (segment_type 0x14).
//!
//! Defines the display resolution and optional display window for a subtitle service.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// The display_definition_segment segment_type as listed in Table 7.
pub const SEGMENT_TYPE: u8 = 0x14;
/// Segment header is sync_byte(1) + segment_type(1) + page_id(2) + segment_length(2) = 6 bytes.
pub const HEADER_LEN: usize = 6;
/// Fixed segment body (after header): dds_version_number(4b)+display_window_flag(1b)+reserved(3b) + display_width(2) + display_height(2) = 5 bytes.
pub const FIXED_BODY_LEN: usize = 5;
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
        if bytes.len() < min {
            return Err(Error::BufferTooShort {
                need: min,
                have: bytes.len(),
                what: "display_definition_segment",
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
                what: "display_definition_segment data",
            });
        }
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        if body.len() < FIXED_BODY_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_BODY_LEN,
                have: body.len(),
                what: "display_definition_segment body",
            });
        }
        let b0 = body[0];
        let dds_version_number = b0 >> 4;
        let display_window_flag = (b0 & 0x08) != 0;
        let reserved = b0 & 0x07;
        let display_width = u16::from_be_bytes([body[1], body[2]]);
        let display_height = u16::from_be_bytes([body[3], body[4]]);

        if display_window_flag {
            if body.len() < FIXED_BODY_LEN + WINDOW_LEN {
                return Err(Error::BufferTooShort {
                    need: FIXED_BODY_LEN + WINDOW_LEN,
                    have: body.len(),
                    what: "display_definition_segment window",
                });
            }
            let dw = &body[FIXED_BODY_LEN..FIXED_BODY_LEN + WINDOW_LEN];
            Ok(DisplayDefinitionSegment {
                page_id,
                dds_version_number,
                display_window_flag,
                reserved,
                display_width,
                display_height,
                display_window_horizontal_position_minimum: Some(u16::from_be_bytes([
                    dw[0], dw[1],
                ])),
                display_window_horizontal_position_maximum: Some(u16::from_be_bytes([
                    dw[2], dw[3],
                ])),
                display_window_vertical_position_minimum: Some(u16::from_be_bytes([dw[4], dw[5]])),
                display_window_vertical_position_maximum: Some(u16::from_be_bytes([dw[6], dw[7]])),
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
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "display_definition_segment serialize",
            });
        }
        // Generic header
        buf[0] = 0x0F; // sync_byte
        buf[1] = SEGMENT_TYPE;
        buf[2..4].copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = (len - HEADER_LEN) as u16;
        buf[4..6].copy_from_slice(&seg_len.to_be_bytes());

        // Fixed body byte 0: version(4b) | window_flag(1b) | reserved(3b)
        buf[6] = (self.dds_version_number << 4)
            | (u8::from(self.display_window_flag) << 3)
            | (self.reserved & 0x07);
        // Fixed body bytes 1-4: display_width, display_height
        buf[7..9].copy_from_slice(&self.display_width.to_be_bytes());
        buf[9..11].copy_from_slice(&self.display_height.to_be_bytes());

        // Optional window
        if self.display_window_flag {
            let wmin = self.display_window_horizontal_position_minimum.unwrap_or(0);
            let wmax = self.display_window_horizontal_position_maximum.unwrap_or(0);
            let vmin = self.display_window_vertical_position_minimum.unwrap_or(0);
            let vmax = self.display_window_vertical_position_maximum.unwrap_or(0);
            buf[11..13].copy_from_slice(&wmin.to_be_bytes());
            buf[13..15].copy_from_slice(&wmax.to_be_bytes());
            buf[15..17].copy_from_slice(&vmin.to_be_bytes());
            buf[17..19].copy_from_slice(&vmax.to_be_bytes());
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::{Parse, Serialize};

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
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test: mutation changes output
        let mut seg2 = seg.clone();
        seg2.display_width = 100;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = DisplayDefinitionSegment::parse(&out2).unwrap();
        assert_eq!(reparse.display_width, 100);
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
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test: clearing the flag drops window
        let mut seg2 = seg.clone();
        seg2.display_window_flag = false;
        let out2 = seg2.to_bytes();
        assert_ne!(out2.len(), out.len());
        let reparse = DisplayDefinitionSegment::parse(&out2).unwrap();
        assert!(!reparse.display_window_flag);
    }
}
