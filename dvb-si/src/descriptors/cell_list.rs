//! Cell List Descriptor — ETSI EN 300 468 §6.2.7 (tag 0x6C, Table 24, PDF p. 58).
//!
//! Carried inside the NIT. Describes the geographic cells of a terrestrial
//! network and their sub-cells. Body layout (Table 24):
//!
//! ```text
//! for (i=0;i<N;i++) {
//!   cell_id                  16
//!   cell_latitude            16
//!   cell_longitude           16
//!   cell_extent_of_latitude  12
//!   cell_extent_of_longitude 12
//!   subcell_info_loop_length  8
//!   for (j=0;j<N;j++) {
//!     cell_id_extension       8
//!     subcell_latitude        16
//!     subcell_longitude       16
//!     subcell_extent_of_latitude  12
//!     subcell_extent_of_longitude 12
//!   }
//! }
//! ```
//!
//! Both loops are typed. The two 12-bit extents pack into 3 bytes (24 bits);
//! they are exposed as `u16` with the high 4 bits unused. latitude/longitude
//! are kept as the raw 16-bit wire value (spec: 16-bit two's-complement
//! fractions of 90°/180°; we preserve the bits verbatim, no scaling).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for cell_list_descriptor.
pub const TAG: u8 = 0x6C;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Bytes per outer entry before the subcell loop:
/// cell_id(2)+lat(2)+long(2)+extents(3)+loop_len(1) = 10.
pub const OUTER_FIXED_LEN: usize = 10;
/// Bytes per subcell entry: ext(1)+lat(2)+long(2)+extents(3) = 8.
pub const SUBCELL_LEN: usize = 8;
/// Mask for a 12-bit extent value.
pub const EXTENT_MASK: u16 = 0x0FFF;

/// One sub-cell within a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CellListSubcell {
    /// 8-bit cell_id_extension.
    pub cell_id_extension: u8,
    /// 16-bit subcell_latitude (raw wire value).
    pub subcell_latitude: u16,
    /// 16-bit subcell_longitude (raw wire value).
    pub subcell_longitude: u16,
    /// 12-bit subcell_extent_of_latitude.
    pub subcell_extent_of_latitude: u16,
    /// 12-bit subcell_extent_of_longitude.
    pub subcell_extent_of_longitude: u16,
}

/// One cell with its sub-cell list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CellListEntry {
    /// 16-bit cell_id.
    pub cell_id: u16,
    /// 16-bit cell_latitude (raw wire value).
    pub cell_latitude: u16,
    /// 16-bit cell_longitude (raw wire value).
    pub cell_longitude: u16,
    /// 12-bit cell_extent_of_latitude.
    pub cell_extent_of_latitude: u16,
    /// 12-bit cell_extent_of_longitude.
    pub cell_extent_of_longitude: u16,
    /// Sub-cells for this cell.
    pub subcells: Vec<CellListSubcell>,
}

/// Cell List Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CellListDescriptor {
    /// Outer cell entries in wire order.
    pub entries: Vec<CellListEntry>,
}

/// Decode the packed 24-bit extent pair `lat(12) || long(12)` from 3 bytes.
fn read_extents(b: &[u8]) -> (u16, u16) {
    let lat = (u16::from(b[0]) << 4) | (u16::from(b[1]) >> 4);
    let long = ((u16::from(b[1]) & 0x0F) << 8) | u16::from(b[2]);
    (lat, long)
}

/// Encode a packed 24-bit extent pair into 3 bytes.
fn write_extents(buf: &mut [u8], lat: u16, long: u16) {
    let lat = lat & EXTENT_MASK;
    let long = long & EXTENT_MASK;
    buf[0] = (lat >> 4) as u8;
    buf[1] = (((lat & 0x0F) << 4) | (long >> 8)) as u8;
    buf[2] = long as u8;
}

impl<'a> Parse<'a> for CellListDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "CellListDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for cell_list_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "CellListDescriptor body",
            });
        }
        let body = &bytes[HEADER_LEN..end];
        let mut entries = Vec::new();
        let mut pos = 0;
        while pos < body.len() {
            if pos + OUTER_FIXED_LEN > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "cell_list outer entry truncated",
                });
            }
            let cell_id = u16::from_be_bytes([body[pos], body[pos + 1]]);
            let cell_latitude = u16::from_be_bytes([body[pos + 2], body[pos + 3]]);
            let cell_longitude = u16::from_be_bytes([body[pos + 4], body[pos + 5]]);
            let (cell_extent_of_latitude, cell_extent_of_longitude) =
                read_extents(&body[pos + 6..pos + 9]);
            let subcell_info_loop_length = body[pos + 9] as usize;
            pos += OUTER_FIXED_LEN;
            if subcell_info_loop_length % SUBCELL_LEN != 0 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "subcell_info_loop_length must be a multiple of 8",
                });
            }
            if pos + subcell_info_loop_length > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "subcell_info_loop_length exceeds descriptor body",
                });
            }
            let subcell_count = subcell_info_loop_length / SUBCELL_LEN;
            let mut subcells = Vec::with_capacity(subcell_count);
            for _ in 0..subcell_count {
                let cell_id_extension = body[pos];
                let subcell_latitude = u16::from_be_bytes([body[pos + 1], body[pos + 2]]);
                let subcell_longitude = u16::from_be_bytes([body[pos + 3], body[pos + 4]]);
                let (subcell_extent_of_latitude, subcell_extent_of_longitude) =
                    read_extents(&body[pos + 5..pos + 8]);
                subcells.push(CellListSubcell {
                    cell_id_extension,
                    subcell_latitude,
                    subcell_longitude,
                    subcell_extent_of_latitude,
                    subcell_extent_of_longitude,
                });
                pos += SUBCELL_LEN;
            }
            entries.push(CellListEntry {
                cell_id,
                cell_latitude,
                cell_longitude,
                cell_extent_of_latitude,
                cell_extent_of_longitude,
                subcells,
            });
        }
        Ok(Self { entries })
    }
}

impl CellListDescriptor {
    fn body_len(&self) -> usize {
        self.entries
            .iter()
            .map(|e| OUTER_FIXED_LEN + e.subcells.len() * SUBCELL_LEN)
            .sum()
    }
}

impl Serialize for CellListDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.body_len();
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "cell_list_descriptor body exceeds 255 bytes",
            });
        }
        for e in &self.entries {
            if e.subcells.len() * SUBCELL_LEN > u8::MAX as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "subcell_info_loop_length exceeds 255 bytes",
                });
            }
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            buf[pos..pos + 2].copy_from_slice(&e.cell_id.to_be_bytes());
            buf[pos + 2..pos + 4].copy_from_slice(&e.cell_latitude.to_be_bytes());
            buf[pos + 4..pos + 6].copy_from_slice(&e.cell_longitude.to_be_bytes());
            write_extents(
                &mut buf[pos + 6..pos + 9],
                e.cell_extent_of_latitude,
                e.cell_extent_of_longitude,
            );
            buf[pos + 9] = (e.subcells.len() * SUBCELL_LEN) as u8;
            pos += OUTER_FIXED_LEN;
            for sc in &e.subcells {
                buf[pos] = sc.cell_id_extension;
                buf[pos + 1..pos + 3].copy_from_slice(&sc.subcell_latitude.to_be_bytes());
                buf[pos + 3..pos + 5].copy_from_slice(&sc.subcell_longitude.to_be_bytes());
                write_extents(
                    &mut buf[pos + 5..pos + 8],
                    sc.subcell_extent_of_latitude,
                    sc.subcell_extent_of_longitude,
                );
                pos += SUBCELL_LEN;
            }
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for CellListDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        self.body_len() as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for CellListDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CELL_LIST";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extents_pack_round_trips() {
        let mut b = [0u8; 3];
        write_extents(&mut b, 0xABC, 0xDEF);
        assert_eq!(read_extents(&b), (0xABC, 0xDEF));
    }

    #[test]
    fn parse_entry_with_subcell() {
        let bytes = [
            TAG, 18, // body length: outer(10) + 1 subcell(8)
            // outer: cell_id=0x1234, lat=0x5678, long=0x9ABC, extent_lat=0xDEF, extent_long=0x012, loop_len=8
            0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x12, 8,
            // subcell: ext=0x07, lat=0x1111, long=0x2222, extent_lat=0x333, extent_long=0x444
            0x07, 0x11, 0x11, 0x22, 0x22, 0x33, 0x34, 0x44,
        ];
        let d = CellListDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        let e = &d.entries[0];
        assert_eq!(e.cell_id, 0x1234);
        assert_eq!(e.cell_latitude, 0x5678);
        assert_eq!(e.cell_longitude, 0x9ABC);
        assert_eq!(e.cell_extent_of_latitude, 0xDEF);
        assert_eq!(e.cell_extent_of_longitude, 0x012);
        assert_eq!(e.subcells.len(), 1);
        let sc = &e.subcells[0];
        assert_eq!(sc.cell_id_extension, 0x07);
        assert_eq!(sc.subcell_latitude, 0x1111);
        assert_eq!(sc.subcell_longitude, 0x2222);
        assert_eq!(sc.subcell_extent_of_latitude, 0x333);
        assert_eq!(sc.subcell_extent_of_longitude, 0x444);
    }

    #[test]
    fn parse_entry_no_subcells() {
        let bytes = [
            TAG, 10, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let d = CellListDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert!(d.entries[0].subcells.is_empty());
    }

    #[test]
    fn empty_body_is_valid() {
        let d = CellListDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            CellListDescriptor::parse(&[0x6D, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x6D, .. }
        ));
    }

    #[test]
    fn parse_rejects_truncated_outer() {
        let bytes = [TAG, 5, 0, 0, 0, 0, 0];
        assert!(matches!(
            CellListDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_subcell_loop_overrun() {
        // loop_len=8 but no subcell bytes follow
        let bytes = [
            TAG, 10, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 8,
        ];
        assert!(matches!(
            CellListDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_buffer_shorter_than_length() {
        let bytes = [TAG, 10, 0x00, 0x01, 0x00];
        assert!(matches!(
            CellListDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = CellListDescriptor {
            entries: vec![
                CellListEntry {
                    cell_id: 0x1234,
                    cell_latitude: 0x5678,
                    cell_longitude: 0x9ABC,
                    cell_extent_of_latitude: 0xDEF,
                    cell_extent_of_longitude: 0x012,
                    subcells: vec![CellListSubcell {
                        cell_id_extension: 0x07,
                        subcell_latitude: 0x1111,
                        subcell_longitude: 0x2222,
                        subcell_extent_of_latitude: 0x333,
                        subcell_extent_of_longitude: 0x444,
                    }],
                },
                CellListEntry {
                    cell_id: 0xAAAA,
                    cell_latitude: 0xBBBB,
                    cell_longitude: 0xCCCC,
                    cell_extent_of_latitude: 0x111,
                    cell_extent_of_longitude: 0x222,
                    subcells: vec![],
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(CellListDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = CellListDescriptor {
            entries: vec![CellListEntry {
                cell_id: 0,
                cell_latitude: 0,
                cell_longitude: 0,
                cell_extent_of_latitude: 0,
                cell_extent_of_longitude: 0,
                subcells: vec![],
            }],
        };
        let mut buf = vec![0u8; 3];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        // 26 empty entries × 10 = 260 > 255.
        let d = CellListDescriptor {
            entries: (0..26)
                .map(|_| CellListEntry {
                    cell_id: 0,
                    cell_latitude: 0,
                    cell_longitude: 0,
                    cell_extent_of_latitude: 0,
                    cell_extent_of_longitude: 0,
                    subcells: vec![],
                })
                .collect(),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = CellListDescriptor {
            entries: vec![CellListEntry {
                cell_id: 0x1234,
                cell_latitude: 0x5678,
                cell_longitude: 0x9ABC,
                cell_extent_of_latitude: 0xDEF,
                cell_extent_of_longitude: 0x012,
                subcells: vec![CellListSubcell {
                    cell_id_extension: 0x07,
                    subcell_latitude: 0x1111,
                    subcell_longitude: 0x2222,
                    subcell_extent_of_latitude: 0x333,
                    subcell_extent_of_longitude: 0x444,
                }],
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
