//! Content Identifier Table — ETSI TS 102 323 v1.4.1 §12.2.
//!
//! The CIT maps content reference identifiers (CRIDs) to events for a given
//! service. Carried on PID 0x0012 (shared with the EIT) with table_id 0x77.
//! Structure: fixed header + prepend-string block + typed CRID entry loop + CRC-32.
//!
//! The CRID entry loop is unfolded into [`CridEntry`] instances (Table 119,
//! §12.2). The prepend-string block is a flat byte array of null-terminated
//! fragments addressed by index; it is kept raw (`&[u8]`) since each entry is
//! just a single byte — there is no per-entry sub-structure to unfold.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// `table_id` for Content Identifier Table.
pub const TABLE_ID: u8 = 0x77;

/// PID on which CIT sections are carried.
///
/// Note: PID 0x0012 is shared with the EIT family; demultiplexers must filter
/// by table_id in addition to PID to isolate CIT sections.
pub const PID: u16 = 0x0012;

const HEADER_LEN: usize = 3;
const EXTENSION_LEN: usize = 10;
const CRC_LEN: usize = 4;
const MIN_SECTION_LEN: usize = HEADER_LEN + EXTENSION_LEN + CRC_LEN;

const CRID_REF_LEN: usize = 2;
const CRID_ENTRY_FIXED_LEN: usize = CRID_REF_LEN + 1 + 1;

/// A single CRID entry in the CIT loop (Table 119, §12.2).
///
/// Wire layout: `crid_ref(16) | prepend_string_index(8) | unique_string_length(8)
/// | unique_string_byte[8]×unique_string_length`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CridEntry<'a> {
    /// `crid_ref` — 16-bit reference into the CRID resolution system.
    pub crid_ref: u16,
    /// `prepend_string_index` — index into the prepend-string block.
    /// `0xFF` means no prepend string (the unique string is the full CRID).
    pub prepend_string_index: u8,
    /// `unique_string` — the unique portion of the CRID (borrowed from the
    /// input buffer).
    pub unique_string: &'a [u8],
}

/// Content Identifier Table (ETSI TS 102 323 v1.4.1 §12.2, Table 119).
///
/// The `crid_entries` loop is unfolded into typed [`CridEntry`] instances.
/// The `prepend_strings` block is kept as a raw byte slice (flat array of
/// null-terminated fragments with no per-entry sub-structure to type).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CitSection<'a> {
    /// `private_indicator` bit from byte 1.
    pub private_indicator: bool,
    /// `service_id` — identifies the container this section belongs to
    /// (table_id_extension, bytes 3-4).
    pub service_id: u16,
    /// 5-bit `version_number`.
    pub version_number: u8,
    /// `current_next_indicator` bit.
    pub current_next_indicator: bool,
    /// Section counter within the sub-table.
    pub section_number: u8,
    /// Final section number in the sub-table.
    pub last_section_number: u8,
    /// `transport_stream_id` of the carrying TS.
    pub transport_stream_id: u16,
    /// `original_network_id` of the originating network.
    pub original_network_id: u16,
    /// Raw prepend-string block (null-terminated fragments addressed by index).
    /// The wire `prepend_strings_length` byte is derived from
    /// `prepend_strings.len()` on serialize (≤ 255).
    pub prepend_strings: &'a [u8],
    /// CRID entry loop — unfolded per Table 119.
    pub crid_entries: Vec<CridEntry<'a>>,
}

impl<'a> CitSection<'a> {
    /// Resolve a prepend string by its `prepend_string_index`.
    ///
    /// Returns `None` if `index` is out of range. The block is a sequence of
    /// null-terminated fragments; index 0 is the first fragment, index 1 the
    /// second, etc. The returned slice includes everything up to (but not
    /// including) the terminating NUL byte; an empty slice means the index
    /// points to an empty or immediately-terminated fragment.
    pub fn prepend_string(&self, index: u8) -> Option<&'a [u8]> {
        let mut remaining = self.prepend_strings;
        let mut current: u8 = 0;
        while !remaining.is_empty() {
            let nul_pos = remaining
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(remaining.len());
            let fragment = &remaining[..nul_pos];
            if current == index {
                return Some(fragment);
            }
            current += 1;
            remaining = if nul_pos < remaining.len() {
                &remaining[nul_pos + 1..]
            } else {
                &[]
            };
        }
        None
    }
}

fn crid_entry_serialized_len(e: &CridEntry) -> usize {
    CRID_ENTRY_FIXED_LEN + e.unique_string.len()
}

impl<'a> Parse<'a> for CitSection<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "CitSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "CitSection",
                expected: &[TABLE_ID],
            });
        }

        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total =
            super::check_section_length(bytes.len(), HEADER_LEN, section_length, MIN_SECTION_LEN)?;

        let private_indicator = (bytes[1] & 0x40) != 0;
        let service_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let transport_stream_id = u16::from_be_bytes([bytes[8], bytes[9]]);
        let original_network_id = u16::from_be_bytes([bytes[10], bytes[11]]);
        let prepend_strings_length = bytes[12];

        let ps_start = HEADER_LEN + EXTENSION_LEN;
        let ps_end = ps_start + prepend_strings_length as usize;
        let payload_end = total - CRC_LEN;
        if ps_end > payload_end {
            return Err(Error::SectionLengthOverflow {
                declared: prepend_strings_length as usize,
                available: payload_end.saturating_sub(ps_start),
            });
        }
        let prepend_strings = &bytes[ps_start..ps_end];

        let mut pos = ps_end;
        let mut crid_entries = Vec::new();
        while pos < payload_end {
            if pos + CRID_ENTRY_FIXED_LEN > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + CRID_ENTRY_FIXED_LEN,
                    have: payload_end,
                    what: "CitSection crid_entry",
                });
            }
            let crid_ref = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
            let prepend_string_index = bytes[pos + 2];
            let unique_string_length = bytes[pos + 3] as usize;
            pos += CRID_ENTRY_FIXED_LEN;
            if pos + unique_string_length > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + unique_string_length,
                    have: payload_end,
                    what: "CitSection unique_string",
                });
            }
            let unique_string = &bytes[pos..pos + unique_string_length];
            pos += unique_string_length;
            crid_entries.push(CridEntry {
                crid_ref,
                prepend_string_index,
                unique_string,
            });
        }

        Ok(CitSection {
            private_indicator,
            service_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            transport_stream_id,
            original_network_id,
            prepend_strings,
            crid_entries,
        })
    }
}

impl Serialize for CitSection<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + EXTENSION_LEN
            + self.prepend_strings.len()
            + self
                .crid_entries
                .iter()
                .map(crid_entry_serialized_len)
                .sum::<usize>()
            + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.prepend_strings.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.prepend_strings.len(),
                available: u8::MAX as usize,
            });
        }

        let section_length = (len - HEADER_LEN) as u16;
        if section_length > 0x0FFF {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: 0x0FFF,
            });
        }
        buf[0] = TABLE_ID;
        buf[1] = super::SECTION_B1_SSI
            | (u8::from(self.private_indicator) << 6)
            | super::SECTION_B1_RESERVED_HI
            | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        buf[3..5].copy_from_slice(&self.service_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8..10].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[10..12].copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[12] = self.prepend_strings.len() as u8;

        let ps_start = HEADER_LEN + EXTENSION_LEN;
        let ps_end = ps_start + self.prepend_strings.len();
        buf[ps_start..ps_end].copy_from_slice(self.prepend_strings);

        let mut pos = ps_end;
        for entry in &self.crid_entries {
            buf[pos..pos + 2].copy_from_slice(&entry.crid_ref.to_be_bytes());
            buf[pos + 2] = entry.prepend_string_index;
            buf[pos + 3] = entry.unique_string.len() as u8;
            pos += CRID_ENTRY_FIXED_LEN;
            buf[pos..pos + entry.unique_string.len()].copy_from_slice(entry.unique_string);
            pos += entry.unique_string.len();
        }

        let crc = dvb_common::crc32_mpeg2::compute(&buf[..pos]);
        buf[pos..pos + CRC_LEN].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for CitSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "CONTENT_IDENTIFIER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_happy_path_no_crid_entries() {
        let prepend = b"CRID://example.com\x00";
        let cit = CitSection {
            private_indicator: false,
            service_id: 0x1234,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            transport_stream_id: 0x0064,
            original_network_id: 0x0002,
            prepend_strings: prepend,
            crid_entries: Vec::new(),
        };
        let mut buf = vec![0u8; cit.serialized_len()];
        cit.serialize_into(&mut buf).unwrap();
        let parsed = CitSection::parse(&buf).unwrap();
        assert_eq!(parsed.service_id, 0x1234);
        assert_eq!(parsed.version_number, 3);
        assert!(parsed.current_next_indicator);
        assert_eq!(parsed.prepend_strings, prepend);
        assert!(parsed.crid_entries.is_empty());
    }

    #[test]
    fn parse_happy_path_with_crid_entries() {
        let prepend = b"crid://bbc.co.uk/\x00";
        let entries = vec![
            CridEntry {
                crid_ref: 0x0001,
                prepend_string_index: 0x00,
                unique_string: b"ep1",
            },
            CridEntry {
                crid_ref: 0x0002,
                prepend_string_index: 0xFF,
                unique_string: b"crid://bbc.co.uk/EV-1",
            },
        ];
        let cit = CitSection {
            private_indicator: false,
            service_id: 0xABCD,
            version_number: 7,
            current_next_indicator: true,
            section_number: 1,
            last_section_number: 3,
            transport_stream_id: 0x01F4,
            original_network_id: 0x0028,
            prepend_strings: prepend,
            crid_entries: entries,
        };
        let mut buf = vec![0u8; cit.serialized_len()];
        cit.serialize_into(&mut buf).unwrap();
        let parsed = CitSection::parse(&buf).unwrap();
        assert_eq!(parsed.service_id, 0xABCD);
        assert_eq!(parsed.crid_entries.len(), 2);
        assert_eq!(parsed.crid_entries[0].crid_ref, 0x0001);
        assert_eq!(parsed.crid_entries[0].prepend_string_index, 0x00);
        assert_eq!(parsed.crid_entries[0].unique_string, b"ep1");
        assert_eq!(parsed.crid_entries[1].crid_ref, 0x0002);
        assert_eq!(parsed.crid_entries[1].prepend_string_index, 0xFF);
        assert_eq!(
            parsed.crid_entries[1].unique_string,
            b"crid://bbc.co.uk/EV-1"
        );
    }

    #[test]
    fn byte_exact_round_trip() {
        let prepend = b"crid://example.com/\x00";
        let entries = vec![CridEntry {
            crid_ref: 0x0042,
            prepend_string_index: 0x00,
            unique_string: b"episode42",
        }];
        let original = CitSection {
            private_indicator: true,
            service_id: 0x4321,
            version_number: 15,
            current_next_indicator: false,
            section_number: 2,
            last_section_number: 4,
            transport_stream_id: 0x03E8,
            original_network_id: 0x0050,
            prepend_strings: prepend,
            crid_entries: entries,
        };
        let mut buf = vec![0u8; original.serialized_len()];
        original.serialize_into(&mut buf).unwrap();
        let parsed = CitSection::parse(&buf).unwrap();
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "byte-exact re-serialize");
        assert_eq!(parsed.crid_entries.len(), 1);
        assert_eq!(parsed.crid_entries[0].crid_ref, 0x0042);
        assert_eq!(parsed.crid_entries[0].unique_string, b"episode42");
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let cit = CitSection {
            private_indicator: false,
            service_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            transport_stream_id: 0x0001,
            original_network_id: 0x0001,
            prepend_strings: &[],
            crid_entries: Vec::new(),
        };
        let mut buf = vec![0u8; cit.serialized_len()];
        cit.serialize_into(&mut buf).unwrap();
        buf[0] = 0x40;
        assert!(matches!(
            CitSection::parse(&buf).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x40, .. }
        ));
    }

    #[test]
    fn parse_rejects_buffer_too_short() {
        assert!(matches!(
            CitSection::parse(&[TABLE_ID, 0x00]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_truncated_crid_entry() {
        let prepend: &[u8] = &[];
        let cit = CitSection {
            private_indicator: false,
            service_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            transport_stream_id: 0x0001,
            original_network_id: 0x0001,
            prepend_strings: prepend,
            crid_entries: Vec::new(),
        };
        let mut buf = vec![0u8; cit.serialized_len()];
        cit.serialize_into(&mut buf).unwrap();
        let mut truncated = buf.clone();
        truncated.truncate(buf.len() - 2);
        let sl = (truncated.len() - HEADER_LEN) as u16;
        truncated[1] = (truncated[1] & 0xF0) | ((sl >> 8) as u8 & 0x0F);
        truncated[2] = (sl & 0xFF) as u8;
        assert!(CitSection::parse(&truncated).is_err());
    }

    #[test]
    fn serialize_rejects_output_buffer_too_small() {
        let cit = CitSection {
            private_indicator: false,
            service_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            transport_stream_id: 0x0001,
            original_network_id: 0x0001,
            prepend_strings: &[],
            crid_entries: Vec::new(),
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            cit.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            CitSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn parse_handwritten_cit_no_entries() {
        let mut bytes: Vec<u8> = vec![
            0x77, 0xF0, 0x0E, 0x12, 0x34, 0xC7, 0x00, 0x00, 0x00, 0x64, 0x00, 0x02, 0x00,
        ];
        let crc = dvb_common::crc32_mpeg2::compute(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        let cit = CitSection::parse(&bytes).unwrap();
        assert_eq!(cit.service_id, 0x1234);
        assert_eq!(cit.transport_stream_id, 0x0064);
        assert!(cit.crid_entries.is_empty());
    }

    #[test]
    fn prepend_string_resolver() {
        let cit = CitSection {
            private_indicator: false,
            service_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            transport_stream_id: 0x0001,
            original_network_id: 0x0001,
            prepend_strings: b"crid://example.com/\x00crid://other.com/\x00",
            crid_entries: Vec::new(),
        };
        assert_eq!(cit.prepend_string(0), Some(&b"crid://example.com/"[..]));
        assert_eq!(cit.prepend_string(1), Some(&b"crid://other.com/"[..]));
        assert_eq!(cit.prepend_string(2), None);
    }
}
