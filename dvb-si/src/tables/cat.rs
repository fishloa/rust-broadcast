//! Conditional Access Table — ISO/IEC 13818-1 §2.4.4.6.
//!
//! Carried on PID 0x0001 with table_id 0x01. Contains a flat
//! list of CA descriptors (tag 0x09) identifying every CA system
//! in use plus the EMM PID on which Entitlement Management
//! Messages for that system are carried.
//!
//! A single-section table per CAS standard.

use crate::descriptors::ca::CaDescriptor;
use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

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
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Cat<'a> {
    /// 5-bit version_number from the section header.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence (typically 0 — single-section).
    pub section_number: u8,
    /// last_section_number (typically 0).
    pub last_section_number: u8,
    /// Descriptor loop (byte 8 → CRC), preserved verbatim. ISO/IEC 13818-1
    /// §2.4.4.6 permits descriptors other than CA in this loop; keeping the loop
    /// raw makes parse → serialize identity hold for all of them. Serializes as
    /// the typed descriptor sequence; `.raw()` yields the wire bytes. Use
    /// [`Cat::ca_descriptors`] for the typed CA (tag 0x09) view.
    pub descriptors: DescriptorLoop<'a>,
}

impl<'a> Cat<'a> {
    /// Typed view of the CA descriptors (tag 0x09) in the descriptor loop.
    /// Non-CA descriptors are skipped; a truncated trailing descriptor ends
    /// the walk.
    #[must_use]
    pub fn ca_descriptors(&self) -> Vec<CatCaEntry> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos + 2 <= self.descriptors.len() {
            let tag = self.descriptors[pos];
            let length = self.descriptors[pos + 1] as usize;
            let end = pos + 2 + length;
            if end > self.descriptors.len() {
                break;
            }
            if tag == crate::descriptors::ca::TAG {
                if let Ok(ca) = CaDescriptor::parse(&self.descriptors[pos..end]) {
                    out.push(CatCaEntry {
                        ca_system_id: ca.ca_system_id,
                        ca_pid: ca.ca_pid,
                        private_data: ca.private_data.to_vec(),
                    });
                }
            }
            pos = end;
        }
        out
    }
}

impl<'a> Parse<'a> for Cat<'a> {
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

        // Descriptor loop runs from byte 8 up to (but not including) the 4-byte
        // CRC. Kept raw — see the field doc; typed CA view via ca_descriptors().
        let descriptors_end = total - CRC_LEN;

        Ok(Cat {
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            descriptors: DescriptorLoop::new(&bytes[8..descriptors_end]),
        })
    }
}

impl Serialize for Cat<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN + EXTENSION_HEADER_LEN + self.descriptors.len() + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length = (len - MIN_HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        // table_id_extension is reserved for the CAT — conventionally 0xFFFF.
        buf[3] = 0xFF;
        buf[4] = 0xFF;
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        let desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        buf[desc_start..desc_start + self.descriptors.len()]
            .copy_from_slice(self.descriptors.raw());
        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for Cat<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Cat<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "CONDITIONAL_ACCESS";
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
        assert!(cat.descriptors.is_empty());
        assert_eq!(cat.ca_descriptors().len(), 0);
    }

    #[test]
    fn parse_single_ca_descriptor_extracts_caid_and_pid() {
        let mut desc = Vec::new();
        desc.extend_from_slice(&ca_descriptor(0x0500, 0x0050));
        let bytes = build_cat(0, &desc);
        let cat = Cat::parse(&bytes).unwrap();
        let cas = cat.ca_descriptors();
        assert_eq!(cas.len(), 1);
        assert_eq!(cas[0].ca_system_id, 0x0500);
        assert_eq!(cas[0].ca_pid, 0x0050);
        assert!(cas[0].private_data.is_empty());
    }

    #[test]
    fn parse_multiple_ca_descriptors_preserves_order() {
        let mut desc = Vec::new();
        desc.extend_from_slice(&ca_descriptor(0x0500, 0x0050));
        desc.extend_from_slice(&ca_descriptor(0x0650, 0x0062));
        desc.extend_from_slice(&ca_descriptor(0x0100, 0x0080));
        let bytes = build_cat(2, &desc);
        let cat = Cat::parse(&bytes).unwrap();
        let cas = cat.ca_descriptors();
        assert_eq!(cas.len(), 3);
        assert_eq!(cas[0].ca_system_id, 0x0500);
        assert_eq!(cas[1].ca_system_id, 0x0650);
        assert_eq!(cas[2].ca_system_id, 0x0100);
        assert_eq!(cas[1].ca_pid, 0x0062);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_cat(0, &[]);
        bytes[0] = 0x02; // PMT table_id
        let err = Cat::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x02, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Cat::parse(&[0x01, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    /// §2.4.4.6 permits non-CA descriptors in the CAT loop: the typed view
    /// skips them, but parse → serialize MUST preserve them byte-for-byte.
    #[test]
    fn non_ca_descriptors_skipped_by_view_but_round_trip() {
        let mut desc = Vec::new();
        desc.extend_from_slice(&ca_descriptor(0x0500, 0x0050));
        desc.extend_from_slice(&[0x12, 0x02, 0xAA, 0xBB]); // unknown tag 0x12, len 2
        desc.extend_from_slice(&ca_descriptor(0x0650, 0x0062));
        let bytes = build_cat(0, &desc);
        let cat = Cat::parse(&bytes).unwrap();
        let cas = cat.ca_descriptors();
        assert_eq!(cas.len(), 2);
        assert_eq!(cas[0].ca_system_id, 0x0500);
        assert_eq!(cas[1].ca_system_id, 0x0650);
        // The unknown descriptor survives the round trip verbatim.
        assert_eq!(cat.descriptors.raw(), desc);
        let mut buf = vec![0u8; cat.serialized_len()];
        cat.serialize_into(&mut buf).unwrap();
        let re = Cat::parse(&buf).unwrap();
        assert_eq!(re.descriptors.raw(), desc);
    }

    #[test]
    fn serialize_round_trip() {
        let mut desc = Vec::new();
        desc.extend_from_slice(&ca_descriptor(0x0500, 0x0050));
        desc.extend_from_slice(&ca_descriptor(0x0650, 0x0062));
        let bytes = build_cat(3, &desc);
        let cat = Cat::parse(&bytes).unwrap();
        let mut buf = vec![0u8; cat.serialized_len()];
        cat.serialize_into(&mut buf).unwrap();
        assert_eq!(Cat::parse(&buf).unwrap(), cat);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<Cat<'_> as Table>::TABLE_ID, 0x01);
        assert_eq!(<Cat<'_> as Table>::PID, 0x0001);
    }

    /// CAT borrows its descriptor loop (3.0): the loop serializes as the
    /// typed descriptor sequence and the struct is serialize-only. Verify the
    /// CA descriptor decodes inside the JSON.
    #[test]
    fn serde_json_serializes_typed_loop() {
        let bytes = build_cat(1, &ca_descriptor(0x0500, 0x0050));
        let cat = Cat::parse(&bytes).unwrap();
        let v = serde_json::to_value(&cat).unwrap();
        let loop_ = v["descriptors"]
            .as_array()
            .expect("typed descriptor sequence");
        assert_eq!(loop_.len(), 1);
        assert_eq!(loop_[0]["ca"]["ca_system_id"], 0x0500);
        assert_eq!(loop_[0]["ca"]["ca_pid"], 0x0050);
    }
}
