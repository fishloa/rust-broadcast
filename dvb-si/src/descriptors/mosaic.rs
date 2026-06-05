//! Mosaic Descriptor — ETSI EN 300 468 §6.2.21 (tag 0x51).
//!
//! Table 71 (PDF p. 91, verified against the vendored PDF). Describes a mosaic
//! service: a grid of elementary cells, each logical cell grouping one or more
//! elementary cells and optionally linking to a bouquet / service / event.
//!
//! Wire layout:
//!   header byte: mosaic_entry_point(1) | num_horizontal_cells(3)
//!                | reserved(1) | num_vertical_cells(3)
//!   for each logical cell:
//!     2 bytes: logical_cell_id(6) | reserved(7) | logical_cell_presentation_info(3)
//!     elementary_cell_field_length(8)
//!     elementary_cell_field_length × { reserved(2) | elementary_cell_id(6) }
//!     cell_linkage_info(8)
//!     conditional linkage payload (see CellLinkage)
//!
//! The first loop level (logical cells) is typed. Within each cell, the
//! elementary_cell_id list is exposed as `Vec<u8>` of 6-bit ids and the
//! linkage payload as the typed `CellLinkage` enum.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for mosaic_descriptor.
pub const TAG: u8 = 0x51;
const HEADER_LEN: usize = 2;
const GRID_HEADER_LEN: usize = 1;
const CELL_FIXED_LEN: usize = 3; // 2 id/presentation bytes + elementary_cell_field_length
/// Maximum body length expressible in the 8-bit `descriptor_length` field.
const MAX_BODY_LEN: usize = u8::MAX as usize;
/// Maximum elementary_cell_field_length (8-bit field).
const MAX_ELEM_FIELD: usize = u8::MAX as usize;

const ENTRY_POINT_MASK: u8 = 0x80; // header bit 7
const ELEM_CELL_ID_MASK: u8 = 0x3F; // low 6 bits

/// Conditional linkage payload selected by `cell_linkage_info` (Table 75).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum CellLinkage {
    /// 0x00 — undefined (no payload).
    Undefined,
    /// 0x01 — bouquet related.
    Bouquet {
        /// Linked bouquet_id.
        bouquet_id: u16,
    },
    /// 0x02 — service related.
    Service {
        /// Linked original_network_id.
        original_network_id: u16,
        /// Linked transport_stream_id.
        transport_stream_id: u16,
        /// Linked service_id.
        service_id: u16,
    },
    /// 0x03 — other mosaic related.
    OtherMosaic {
        /// Linked original_network_id.
        original_network_id: u16,
        /// Linked transport_stream_id.
        transport_stream_id: u16,
        /// Linked service_id.
        service_id: u16,
    },
    /// 0x04 — event related.
    Event {
        /// Linked original_network_id.
        original_network_id: u16,
        /// Linked transport_stream_id.
        transport_stream_id: u16,
        /// Linked service_id.
        service_id: u16,
        /// Linked event_id.
        event_id: u16,
    },
    /// 0x05..=0xFF — reserved; the raw byte is preserved so unknown linkage
    /// values round-trip (they carry no further payload per Table 75).
    Reserved {
        /// The raw cell_linkage_info byte.
        value: u8,
    },
}

impl CellLinkage {
    /// The cell_linkage_info byte this variant serializes to.
    fn info_byte(&self) -> u8 {
        match self {
            CellLinkage::Undefined => 0x00,
            CellLinkage::Bouquet { .. } => 0x01,
            CellLinkage::Service { .. } => 0x02,
            CellLinkage::OtherMosaic { .. } => 0x03,
            CellLinkage::Event { .. } => 0x04,
            CellLinkage::Reserved { value } => *value,
        }
    }

    /// Length of the conditional payload (excluding the info byte).
    fn payload_len(&self) -> usize {
        match self {
            CellLinkage::Undefined | CellLinkage::Reserved { .. } => 0,
            CellLinkage::Bouquet { .. } => 2,
            CellLinkage::Service { .. } | CellLinkage::OtherMosaic { .. } => 6,
            CellLinkage::Event { .. } => 8,
        }
    }
}

/// One logical cell within the mosaic.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MosaicLogicalCell {
    /// 6-bit logical_cell_id.
    pub logical_cell_id: u8,
    /// 3-bit logical_cell_presentation_info (Table 74): 1 = video,
    /// 2 = still picture, 3 = graphics/text.
    pub presentation_info: u8,
    /// elementary_cell_id values (each 6 bits) composing this logical cell.
    pub elementary_cell_ids: Vec<u8>,
    /// Linkage of this logical cell to a bouquet / service / event.
    pub linkage: CellLinkage,
}

/// Mosaic Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MosaicDescriptor {
    /// mosaic_entry_point: set when this mosaic is the top of a hierarchy.
    pub mosaic_entry_point: bool,
    /// 3-bit number_of_horizontal_elementary_cells (Table 72; 0 = one cell).
    pub num_horizontal_cells: u8,
    /// 3-bit number_of_vertical_elementary_cells (Table 73; 0 = one cell).
    pub num_vertical_cells: u8,
    /// Logical cells in wire order.
    pub logical_cells: Vec<MosaicLogicalCell>,
}

fn read_u16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

impl<'a> Parse<'a> for MosaicDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "MosaicDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for mosaic_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length < GRID_HEADER_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "mosaic_descriptor missing grid header byte",
            });
        }
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "MosaicDescriptor body",
            });
        }
        let body = &bytes[HEADER_LEN..end];
        // grid header: reserved bit (bit 3) ignored on parse (§5.1).
        let mosaic_entry_point = body[0] & ENTRY_POINT_MASK != 0;
        let num_horizontal_cells = (body[0] >> 4) & 0x07;
        let num_vertical_cells = body[0] & 0x07;

        let mut logical_cells = Vec::new();
        let mut pos = GRID_HEADER_LEN;
        while pos < body.len() {
            if pos + CELL_FIXED_LEN > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "truncated mosaic logical cell header",
                });
            }
            // logical_cell_id(6) | reserved(7) | presentation_info(3)
            let logical_cell_id = (body[pos] >> 2) & 0x3F;
            let presentation_info = body[pos + 1] & 0x07;
            let elem_field_len = body[pos + 2] as usize;
            pos += CELL_FIXED_LEN;
            if pos + elem_field_len > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "elementary_cell_field_length exceeds descriptor body",
                });
            }
            let mut elementary_cell_ids = Vec::with_capacity(elem_field_len);
            for &b in &body[pos..pos + elem_field_len] {
                // reserved(2) ignored on parse; low 6 bits are the id.
                elementary_cell_ids.push(b & ELEM_CELL_ID_MASK);
            }
            pos += elem_field_len;
            if pos >= body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "missing cell_linkage_info byte",
                });
            }
            let info = body[pos];
            pos += 1;
            let linkage = match info {
                0x00 => CellLinkage::Undefined,
                0x01 => {
                    if pos + 2 > body.len() {
                        return Err(Error::InvalidDescriptor {
                            tag: TAG,
                            reason: "truncated bouquet linkage payload",
                        });
                    }
                    let l = CellLinkage::Bouquet {
                        bouquet_id: read_u16(body, pos),
                    };
                    pos += 2;
                    l
                }
                0x02 | 0x03 => {
                    if pos + 6 > body.len() {
                        return Err(Error::InvalidDescriptor {
                            tag: TAG,
                            reason: "truncated service/mosaic linkage payload",
                        });
                    }
                    let original_network_id = read_u16(body, pos);
                    let transport_stream_id = read_u16(body, pos + 2);
                    let service_id = read_u16(body, pos + 4);
                    pos += 6;
                    if info == 0x02 {
                        CellLinkage::Service {
                            original_network_id,
                            transport_stream_id,
                            service_id,
                        }
                    } else {
                        CellLinkage::OtherMosaic {
                            original_network_id,
                            transport_stream_id,
                            service_id,
                        }
                    }
                }
                0x04 => {
                    if pos + 8 > body.len() {
                        return Err(Error::InvalidDescriptor {
                            tag: TAG,
                            reason: "truncated event linkage payload",
                        });
                    }
                    let l = CellLinkage::Event {
                        original_network_id: read_u16(body, pos),
                        transport_stream_id: read_u16(body, pos + 2),
                        service_id: read_u16(body, pos + 4),
                        event_id: read_u16(body, pos + 6),
                    };
                    pos += 8;
                    l
                }
                // 0x05..=0xFF reserved: no defined payload (Table 75).
                other => CellLinkage::Reserved { value: other },
            };
            logical_cells.push(MosaicLogicalCell {
                logical_cell_id,
                presentation_info,
                elementary_cell_ids,
                linkage,
            });
        }
        Ok(Self {
            mosaic_entry_point,
            num_horizontal_cells,
            num_vertical_cells,
            logical_cells,
        })
    }
}

impl Serialize for MosaicDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let cells: usize = self
            .logical_cells
            .iter()
            .map(|c| CELL_FIXED_LEN + c.elementary_cell_ids.len() + 1 + c.linkage.payload_len())
            .sum();
        HEADER_LEN + GRID_HEADER_LEN + cells
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let body_len = len - HEADER_LEN;
        // 8-bit descriptor_length field: error rather than silently truncate.
        if body_len > MAX_BODY_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: MAX_BODY_LEN,
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        // grid header: reserved bit (bit 3) emitted as 1 (§5.1).
        buf[2] = if self.mosaic_entry_point {
            ENTRY_POINT_MASK
        } else {
            0
        } | ((self.num_horizontal_cells & 0x07) << 4)
            | 0x08
            | (self.num_vertical_cells & 0x07);
        let mut pos = HEADER_LEN + GRID_HEADER_LEN;
        for cell in &self.logical_cells {
            // 8-bit elementary_cell_field_length: error on over-range.
            if cell.elementary_cell_ids.len() > MAX_ELEM_FIELD {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "elementary_cell_field exceeds 255 entries (8-bit length field)",
                });
            }
            // byte0: logical_cell_id(6) | top 2 reserved bits emitted 1s.
            buf[pos] = ((cell.logical_cell_id & 0x3F) << 2) | 0x03;
            // byte1: 5 reserved bits emitted 1s | presentation_info(3).
            buf[pos + 1] = 0xF8 | (cell.presentation_info & 0x07);
            buf[pos + 2] = cell.elementary_cell_ids.len() as u8;
            pos += CELL_FIXED_LEN;
            for &id in &cell.elementary_cell_ids {
                // reserved(2) emitted 1s | elementary_cell_id(6).
                buf[pos] = 0xC0 | (id & ELEM_CELL_ID_MASK);
                pos += 1;
            }
            buf[pos] = cell.linkage.info_byte();
            pos += 1;
            match &cell.linkage {
                CellLinkage::Undefined | CellLinkage::Reserved { .. } => {}
                CellLinkage::Bouquet { bouquet_id } => {
                    buf[pos..pos + 2].copy_from_slice(&bouquet_id.to_be_bytes());
                    pos += 2;
                }
                CellLinkage::Service {
                    original_network_id,
                    transport_stream_id,
                    service_id,
                }
                | CellLinkage::OtherMosaic {
                    original_network_id,
                    transport_stream_id,
                    service_id,
                } => {
                    buf[pos..pos + 2].copy_from_slice(&original_network_id.to_be_bytes());
                    buf[pos + 2..pos + 4].copy_from_slice(&transport_stream_id.to_be_bytes());
                    buf[pos + 4..pos + 6].copy_from_slice(&service_id.to_be_bytes());
                    pos += 6;
                }
                CellLinkage::Event {
                    original_network_id,
                    transport_stream_id,
                    service_id,
                    event_id,
                } => {
                    buf[pos..pos + 2].copy_from_slice(&original_network_id.to_be_bytes());
                    buf[pos + 2..pos + 4].copy_from_slice(&transport_stream_id.to_be_bytes());
                    buf[pos + 4..pos + 6].copy_from_slice(&service_id.to_be_bytes());
                    buf[pos + 6..pos + 8].copy_from_slice(&event_id.to_be_bytes());
                    pos += 8;
                }
            }
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for MosaicDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for MosaicDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MOSAIC";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_cell_undefined_linkage() {
        // grid header 0x80|0x08 (entry point + reserved), 2x2 grid coded as 1,1
        // logical cell: id=5, presentation=1, 2 elem cells [3,7], linkage 0x00
        let bytes = [
            TAG,
            0x07,
            0x80 | (1 << 4) | 0x08 | 1, // header
            (5 << 2) | 0x03,            // logical_cell_id=5
            0xF8 | 1,                   // presentation_info=1
            0x02,                       // elementary_cell_field_length=2
            0xC0 | 3,
            0xC0 | 7,
            0x00, // cell_linkage_info = undefined
        ];
        let d = MosaicDescriptor::parse(&bytes).unwrap();
        assert!(d.mosaic_entry_point);
        assert_eq!(d.num_horizontal_cells, 1);
        assert_eq!(d.num_vertical_cells, 1);
        assert_eq!(d.logical_cells.len(), 1);
        let c = &d.logical_cells[0];
        assert_eq!(c.logical_cell_id, 5);
        assert_eq!(c.presentation_info, 1);
        assert_eq!(c.elementary_cell_ids, vec![3, 7]);
        assert_eq!(c.linkage, CellLinkage::Undefined);
    }

    #[test]
    fn parse_service_linkage() {
        let d = MosaicDescriptor {
            mosaic_entry_point: false,
            num_horizontal_cells: 0,
            num_vertical_cells: 0,
            logical_cells: vec![MosaicLogicalCell {
                logical_cell_id: 1,
                presentation_info: 2,
                elementary_cell_ids: vec![0],
                linkage: CellLinkage::Service {
                    original_network_id: 0x1111,
                    transport_stream_id: 0x2222,
                    service_id: 0x3333,
                },
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let p = MosaicDescriptor::parse(&buf).unwrap();
        assert_eq!(p, d);
        assert!(matches!(
            p.logical_cells[0].linkage,
            CellLinkage::Service {
                original_network_id: 0x1111,
                transport_stream_id: 0x2222,
                service_id: 0x3333,
            }
        ));
    }

    #[test]
    fn parse_event_linkage_round_trip() {
        let d = MosaicDescriptor {
            mosaic_entry_point: true,
            num_horizontal_cells: 3,
            num_vertical_cells: 2,
            logical_cells: vec![MosaicLogicalCell {
                logical_cell_id: 10,
                presentation_info: 3,
                elementary_cell_ids: vec![1, 2, 3],
                linkage: CellLinkage::Event {
                    original_network_id: 0xAAAA,
                    transport_stream_id: 0xBBBB,
                    service_id: 0xCCCC,
                    event_id: 0xDDDD,
                },
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(MosaicDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn parse_reserved_linkage_round_trips() {
        let bytes = [
            TAG,
            0x05,
            0x08, // grid header, no entry point, grid 0,0
            (1 << 2) | 0x03,
            0xF8,
            0x00, // no elementary cells
            0x42, // reserved linkage value
        ];
        let d = MosaicDescriptor::parse(&bytes).unwrap();
        assert_eq!(
            d.logical_cells[0].linkage,
            CellLinkage::Reserved { value: 0x42 }
        );
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(MosaicDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn parse_multiple_cells() {
        let d = MosaicDescriptor {
            mosaic_entry_point: false,
            num_horizontal_cells: 1,
            num_vertical_cells: 1,
            logical_cells: vec![
                MosaicLogicalCell {
                    logical_cell_id: 0,
                    presentation_info: 1,
                    elementary_cell_ids: vec![0],
                    linkage: CellLinkage::Bouquet { bouquet_id: 0x1234 },
                },
                MosaicLogicalCell {
                    logical_cell_id: 1,
                    presentation_info: 1,
                    elementary_cell_ids: vec![1],
                    linkage: CellLinkage::Undefined,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let p = MosaicDescriptor::parse(&buf).unwrap();
        assert_eq!(p.logical_cells.len(), 2);
        assert_eq!(p, d);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            MosaicDescriptor::parse(&[0x52, 1, 0x08]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x52, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        // declares 5 body bytes, only 2 present
        let bytes = [TAG, 5, 0x08, 0x00];
        assert!(matches!(
            MosaicDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_elem_field_overrun() {
        // elementary_cell_field_length=10 but body has no room
        let bytes = [TAG, 4, 0x08, (1 << 2) | 0x03, 0xF8, 0x0A];
        assert!(matches!(
            MosaicDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_missing_linkage_byte() {
        // cell header present, elem_field_len=0, but no cell_linkage_info byte
        let bytes = [TAG, 4, 0x08, (1 << 2) | 0x03, 0xF8, 0x00];
        // length=4 means body is [0x08, hdr0, hdr1, 0x00]; after the 3 fixed
        // cell bytes pos == body.len() so the linkage byte is missing.
        assert!(matches!(
            MosaicDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_zero_length() {
        assert!(matches!(
            MosaicDescriptor::parse(&[TAG, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn serialize_emits_reserved_ones() {
        let d = MosaicDescriptor {
            mosaic_entry_point: false,
            num_horizontal_cells: 0,
            num_vertical_cells: 0,
            logical_cells: vec![],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // grid header reserved bit (bit 3) set
        assert_eq!(buf[2] & 0x08, 0x08);
    }

    #[test]
    fn serialize_round_trip_empty_grid() {
        let d = MosaicDescriptor {
            mosaic_entry_point: true,
            num_horizontal_cells: 7,
            num_vertical_cells: 7,
            logical_cells: vec![],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(MosaicDescriptor::parse(&buf).unwrap(), d);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = MosaicDescriptor {
            mosaic_entry_point: true,
            num_horizontal_cells: 1,
            num_vertical_cells: 1,
            logical_cells: vec![MosaicLogicalCell {
                logical_cell_id: 5,
                presentation_info: 1,
                elementary_cell_ids: vec![3, 7],
                linkage: CellLinkage::Bouquet { bouquet_id: 0x1234 },
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
