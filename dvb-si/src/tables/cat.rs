//! Conditional Access Table — ISO/IEC 13818-1 §2.4.4.6.
//!
//! Carried on PID 0x0001 with table_id 0x01. Contains a flat
//! list of CA descriptors (tag 0x09) identifying every CA system
//! in use plus the EMM PID on which Entitlement Management
//! Messages for that system are carried.
//!
//! A single-section table per CAS standard.

use crate::descriptors::ca::CaDescriptor;
use crate::error::{Error, Result};
use dvb_common::Parse;

/// CAT table_id (ISO/IEC 13818-1 Table 2-30).
pub const TABLE_ID: u8 = 0x01;
/// CAT well-known PID.
pub const PID: u16 = 0x0001;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const CRC_LEN: usize = 4;

/// One CA descriptor entry from the CAT, in owned form so it
/// outlives the source section bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatCaEntry {
    /// CA System ID — the CAID. e.g. 0x0500 Viaccess, 0x0650
    /// Irdeto ORF-ICE, 0x0100 Seca/Mediaguard.
    pub ca_system_id: u16,
    /// EMM PID for this CA system.
    pub ca_pid: u16,
    /// Optional private data after the standard CA fields.
    pub private_data: Vec<u8>,
}

/// Conditional Access Table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cat {
    /// 5-bit version_number from the section header.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence (typically 0 — single-section).
    pub section_number: u8,
    /// last_section_number (typically 0).
    pub last_section_number: u8,
    /// CA descriptor entries from the section's flat descriptor loop.
    pub ca_descriptors: Vec<CatCaEntry>,
}

impl<'a> Parse<'a> for Cat {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN,
                have: bytes.len(),
                what: "Cat",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Cat",
                expected: &[TABLE_ID],
            });
        }

        let section_length = (((bytes[1] & 0x0F) as u16) << 8) | bytes[2] as u16;
        let total = MIN_HEADER_LEN + section_length as usize;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: bytes.len() - MIN_HEADER_LEN,
            });
        }

        // Skip the 2-byte reserved + extension (bytes 3-4), read version+cni at 5,
        // section/last_section at 6,7. CAT's "table_id_extension" (bytes 3-4) is
        // reserved per spec — we don't expose it.
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        // Descriptor loop runs from byte 8 up to (but not including) the 4-byte CRC.
        let descriptors_end = total - CRC_LEN;
        let mut ca_descriptors = Vec::new();
        let mut pos = 8;
        while pos < descriptors_end {
            // Descriptor header is 2 bytes (tag + length).
            if pos + 2 > descriptors_end {
                break;
            }
            let tag = bytes[pos];
            let length = bytes[pos + 1] as usize;
            let descriptor_end = pos + 2 + length;
            if descriptor_end > descriptors_end {
                // Truncated descriptor — bail out preserving what we already have.
                break;
            }

            // Only CA descriptors (tag 0x09) are meaningful in the CAT loop.
            // The standard does allow other descriptors but in practice all
            // production CATs we've seen carry only CA descriptors.
            if tag == crate::descriptors::ca::TAG {
                if let Ok(ca) = CaDescriptor::parse(&bytes[pos..descriptor_end]) {
                    ca_descriptors.push(CatCaEntry {
                        ca_system_id: ca.ca_system_id,
                        ca_pid: ca.ca_pid,
                        private_data: ca.private_data.to_vec(),
                    });
                }
            }

            pos = descriptor_end;
        }

        Ok(Cat {
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            ca_descriptors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a CAT section with the given CA descriptors and placeholder CRC.
    fn build_cat(version: u8, descriptors: &[u8]) -> Vec<u8> {
        let section_length: u16 =
            (EXTENSION_HEADER_LEN as u16) + descriptors.len() as u16 + (CRC_LEN as u16);
        let mut v = Vec::new();
        v.push(TABLE_ID);
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        // table_id_extension (reserved for CAT) — typically 0xFFFF in the wild.
        v.extend_from_slice(&[0xFF, 0xFF]);
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01); // version + cni=1
        v.push(0x00); // section_number
        v.push(0x00); // last_section_number
        v.extend_from_slice(descriptors);
        v.extend_from_slice(&[0, 0, 0, 0]); // placeholder CRC
        v
    }

    fn ca_descriptor(ca_system_id: u16, ca_pid: u16) -> [u8; 6] {
        [
            0x09,
            0x04,
            (ca_system_id >> 8) as u8,
            (ca_system_id & 0xFF) as u8,
            0xE0 | ((ca_pid >> 8) as u8 & 0x1F),
            (ca_pid & 0xFF) as u8,
        ]
    }

    #[test]
    fn parse_empty_cat_zero_descriptors() {
        let bytes = build_cat(5, &[]);
        let cat = Cat::parse(&bytes).expect("parse");
        assert_eq!(cat.version_number, 5);
        assert!(cat.current_next_indicator);
        assert_eq!(cat.ca_descriptors.len(), 0);
    }

    #[test]
    fn parse_single_ca_descriptor_extracts_caid_and_pid() {
        let mut desc = Vec::new();
        desc.extend_from_slice(&ca_descriptor(0x0500, 0x0050));
        let bytes = build_cat(0, &desc);
        let cat = Cat::parse(&bytes).unwrap();
        assert_eq!(cat.ca_descriptors.len(), 1);
        assert_eq!(cat.ca_descriptors[0].ca_system_id, 0x0500);
        assert_eq!(cat.ca_descriptors[0].ca_pid, 0x0050);
        assert!(cat.ca_descriptors[0].private_data.is_empty());
    }

    #[test]
    fn parse_multiple_ca_descriptors_preserves_order() {
        let mut desc = Vec::new();
        desc.extend_from_slice(&ca_descriptor(0x0500, 0x0050));
        desc.extend_from_slice(&ca_descriptor(0x0650, 0x0062));
        desc.extend_from_slice(&ca_descriptor(0x0100, 0x0080));
        let bytes = build_cat(2, &desc);
        let cat = Cat::parse(&bytes).unwrap();
        assert_eq!(cat.ca_descriptors.len(), 3);
        assert_eq!(cat.ca_descriptors[0].ca_system_id, 0x0500);
        assert_eq!(cat.ca_descriptors[1].ca_system_id, 0x0650);
        assert_eq!(cat.ca_descriptors[2].ca_system_id, 0x0100);
        assert_eq!(cat.ca_descriptors[1].ca_pid, 0x0062);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_cat(0, &[]);
        bytes[0] = 0x02; // PMT table_id
        let err = Cat::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::UnexpectedTableId { table_id: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Cat::parse(&[0x01, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_skips_non_ca_descriptors() {
        // Two CA descriptors with an unknown-tag descriptor between them.
        let mut desc = Vec::new();
        desc.extend_from_slice(&ca_descriptor(0x0500, 0x0050));
        desc.extend_from_slice(&[0x12, 0x02, 0xAA, 0xBB]); // unknown tag 0x12, len 2
        desc.extend_from_slice(&ca_descriptor(0x0650, 0x0062));
        let bytes = build_cat(0, &desc);
        let cat = Cat::parse(&bytes).unwrap();
        assert_eq!(cat.ca_descriptors.len(), 2);
        assert_eq!(cat.ca_descriptors[0].ca_system_id, 0x0500);
        assert_eq!(cat.ca_descriptors[1].ca_system_id, 0x0650);
    }
}
