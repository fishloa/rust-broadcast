//! Cell Frequency Link Descriptor — ETSI EN 300 468 §6.2.7 (tag 0x6D, Table 23, PDF p. 58).
//!
//! Carried inside the NIT. Associates terrestrial cells with their transmit
//! frequency and any transposer (gap-filler) sub-cells. Body layout (Table 23):
//!
//! ```text
//! for (i=0;i<N;i++) {
//!   cell_id                 16
//!   frequency               32
//!   subcell_info_loop_length 8
//!   for (j=0;j<N;j++) {
//!     cell_id_extension      8
//!     transposer_frequency  32
//!   }
//! }
//! ```
//!
//! Both loops are typed. `frequency`/`transposer_frequency` are raw u32 units
//! (4 bytes, 10 Hz units per §6.2.7); we preserve the numeric value verbatim.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for cell_frequency_link_descriptor.
pub const TAG: u8 = 0x6D;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Fixed bytes per outer entry before the subcell loop: cell_id(2)+frequency(4)+loop_len(1).
pub const OUTER_FIXED_LEN: usize = 7;
/// Bytes per subcell entry: cell_id_extension(1)+transposer_frequency(4).
pub const SUBCELL_LEN: usize = 5;

/// One transposer sub-cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CellFrequencyLinkSubcell {
    /// 8-bit cell_id_extension.
    pub cell_id_extension: u8,
    /// 32-bit transposer_frequency (10 Hz units).
    pub transposer_frequency: u32,
}

/// One cell-frequency association with its sub-cell list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CellFrequencyLinkEntry {
    /// 16-bit cell_id.
    pub cell_id: u16,
    /// 32-bit frequency (10 Hz units).
    pub frequency: u32,
    /// Transposer sub-cells for this cell.
    pub subcells: Vec<CellFrequencyLinkSubcell>,
}

/// Cell Frequency Link Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CellFrequencyLinkDescriptor {
    /// Outer cell entries in wire order.
    pub entries: Vec<CellFrequencyLinkEntry>,
}

impl<'a> Parse<'a> for CellFrequencyLinkDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "CellFrequencyLinkDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for cell_frequency_link_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "CellFrequencyLinkDescriptor body",
            });
        }
        let body = &bytes[HEADER_LEN..end];
        let mut entries = Vec::new();
        let mut pos = 0;
        while pos < body.len() {
            if pos + OUTER_FIXED_LEN > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "cell_frequency_link outer entry truncated",
                });
            }
            let cell_id = u16::from_be_bytes([body[pos], body[pos + 1]]);
            let frequency =
                u32::from_be_bytes([body[pos + 2], body[pos + 3], body[pos + 4], body[pos + 5]]);
            let subcell_info_loop_length = body[pos + 6] as usize;
            pos += OUTER_FIXED_LEN;
            if subcell_info_loop_length % SUBCELL_LEN != 0 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "subcell_info_loop_length must be a multiple of 5",
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
                let transposer_frequency = u32::from_be_bytes([
                    body[pos + 1],
                    body[pos + 2],
                    body[pos + 3],
                    body[pos + 4],
                ]);
                subcells.push(CellFrequencyLinkSubcell {
                    cell_id_extension,
                    transposer_frequency,
                });
                pos += SUBCELL_LEN;
            }
            entries.push(CellFrequencyLinkEntry {
                cell_id,
                frequency,
                subcells,
            });
        }
        Ok(Self { entries })
    }
}

impl CellFrequencyLinkDescriptor {
    fn body_len(&self) -> usize {
        self.entries
            .iter()
            .map(|e| OUTER_FIXED_LEN + e.subcells.len() * SUBCELL_LEN)
            .sum()
    }
}

impl Serialize for CellFrequencyLinkDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.body_len();
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "cell_frequency_link_descriptor body exceeds 255 bytes",
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
            buf[pos + 2..pos + 6].copy_from_slice(&e.frequency.to_be_bytes());
            buf[pos + 6] = (e.subcells.len() * SUBCELL_LEN) as u8;
            pos += OUTER_FIXED_LEN;
            for sc in &e.subcells {
                buf[pos] = sc.cell_id_extension;
                buf[pos + 1..pos + 5].copy_from_slice(&sc.transposer_frequency.to_be_bytes());
                pos += SUBCELL_LEN;
            }
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for CellFrequencyLinkDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        self.body_len() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_entry_with_subcells() {
        let bytes = [
            TAG, 17, // body length
            // outer: cell_id=0x1234, freq=0x00112233, subcell_loop_len=10
            0x12, 0x34, 0x00, 0x11, 0x22, 0x33, 10, // subcell 1: ext=0x01, freq=0x0AABBCCD
            0x01, 0x0A, 0xAB, 0xBC, 0xCD, // subcell 2: ext=0x02, freq=0x0DDEEFF0
            0x02, 0x0D, 0xDE, 0xEF, 0xF0,
        ];
        let d = CellFrequencyLinkDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].cell_id, 0x1234);
        assert_eq!(d.entries[0].frequency, 0x0011_2233);
        assert_eq!(d.entries[0].subcells.len(), 2);
        assert_eq!(d.entries[0].subcells[0].cell_id_extension, 0x01);
        assert_eq!(d.entries[0].subcells[0].transposer_frequency, 0x0AAB_BCCD);
        assert_eq!(d.entries[0].subcells[1].cell_id_extension, 0x02);
    }

    #[test]
    fn parse_entry_no_subcells() {
        let bytes = [TAG, 7, 0x00, 0x05, 0x00, 0x00, 0x10, 0x00, 0x00];
        let d = CellFrequencyLinkDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].cell_id, 0x0005);
        assert!(d.entries[0].subcells.is_empty());
    }

    #[test]
    fn empty_body_is_valid() {
        let d = CellFrequencyLinkDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            CellFrequencyLinkDescriptor::parse(&[0x6E, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x6E, .. }
        ));
    }

    #[test]
    fn parse_rejects_truncated_outer() {
        // body length 5 < OUTER_FIXED_LEN(7)
        let bytes = [TAG, 5, 0x00, 0x01, 0x00, 0x00, 0x10];
        assert!(matches!(
            CellFrequencyLinkDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_subcell_loop_overrun() {
        // subcell_loop_len=10 but no subcell bytes follow
        let bytes = [TAG, 7, 0x00, 0x01, 0x00, 0x00, 0x10, 0x00, 10];
        assert!(matches!(
            CellFrequencyLinkDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_buffer_shorter_than_length() {
        let bytes = [TAG, 7, 0x00, 0x01, 0x00];
        assert!(matches!(
            CellFrequencyLinkDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = CellFrequencyLinkDescriptor {
            entries: vec![
                CellFrequencyLinkEntry {
                    cell_id: 0x1234,
                    frequency: 0x0011_2233,
                    subcells: vec![
                        CellFrequencyLinkSubcell {
                            cell_id_extension: 0x01,
                            transposer_frequency: 0x0AAB_BCCD,
                        },
                        CellFrequencyLinkSubcell {
                            cell_id_extension: 0x02,
                            transposer_frequency: 0x0DDE_EFF0,
                        },
                    ],
                },
                CellFrequencyLinkEntry {
                    cell_id: 0x9999,
                    frequency: 0x4455_6677,
                    subcells: vec![],
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(CellFrequencyLinkDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = CellFrequencyLinkDescriptor {
            entries: vec![CellFrequencyLinkEntry {
                cell_id: 0,
                frequency: 0,
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
        // 37 empty entries × 7 bytes = 259 > 255.
        let d = CellFrequencyLinkDescriptor {
            entries: (0..37)
                .map(|_| CellFrequencyLinkEntry {
                    cell_id: 0,
                    frequency: 0,
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
        let d = CellFrequencyLinkDescriptor {
            entries: vec![CellFrequencyLinkEntry {
                cell_id: 0x1234,
                frequency: 0x0011_2233,
                subcells: vec![CellFrequencyLinkSubcell {
                    cell_id_extension: 0x01,
                    transposer_frequency: 0x0AAB_BCCD,
                }],
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: CellFrequencyLinkDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }
}
