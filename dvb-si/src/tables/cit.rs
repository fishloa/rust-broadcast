//! Content Identifier Table — ETSI TS 102 323 v1.4.1 §12.2.
//!
//! The CIT maps content reference identifiers (CRIDs) to events for a given
//! service. Carried on PID 0x0012 (shared with the EIT) with table_id 0x77.
//! Structure: fixed header + prepend-string block + raw CRID entry loop + CRC-32.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for Content Identifier Table.
pub const TABLE_ID: u8 = 0x77;

/// PID on which CIT sections are carried.
///
/// Note: PID 0x0012 is shared with the EIT family; demultiplexers must filter
/// by table_id in addition to PID to isolate CIT sections.
pub const PID: u16 = 0x0012;

// ── length constants ──────────────────────────────────────────────────────────

/// Bytes 0-2: table_id (1) + flags+section_length (2).
const HEADER_LEN: usize = 3;

/// Bytes 3-12: service_id(2) + flags+version+cni(1) + section_number(1)
/// + last_section_number(1) + transport_stream_id(2) + original_network_id(2)
/// + prepend_strings_length(1).
const EXTENSION_LEN: usize = 10;

/// Minimum total encoded length: header + extension + CRC.
const MIN_SECTION_LEN: usize = HEADER_LEN + EXTENSION_LEN + CRC_LEN;

/// Bytes occupied by the trailing CRC-32 field.
const CRC_LEN: usize = 4;

// ── struct ────────────────────────────────────────────────────────────────────

/// Content Identifier Table (ETSI TS 102 323 v1.4.1 §12.2).
///
/// Variable-length fields are kept as raw byte slices borrowing from the source
/// buffer. Callers that need to iterate CRID entries may walk `crid_entries`
/// directly per the wire format:
///
/// ```text
/// for each entry:
///   crid_ref             (2 bytes, u16 big-endian)
///   prepend_string_index (1 byte)
///   unique_string_length (1 byte)
///   unique_string_bytes  (unique_string_length bytes)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Cit<'a> {
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

    /// Raw prepend-string block. The wire `prepend_strings_length` byte is
    /// derived from `prepend_strings.len()` on serialize (≤ 255). Entries are
    /// null-terminated ASCII/DVB-text fragments; addressed by index from the
    /// CRID loop.
    pub prepend_strings: &'a [u8],

    /// Raw CRID entry loop (everything between the prepend-string block and the
    /// CRC-32). Walk per the format documented on the struct.
    pub crid_entries: &'a [u8],
}

// ── Parse ─────────────────────────────────────────────────────────────────────

impl<'a> Parse<'a> for Cit<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // Minimum-length guard.
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "Cit",
            });
        }

        // table_id check.
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Cit",
                expected: &[TABLE_ID],
            });
        }

        // section_length: lower 4 bits of byte 1 || byte 2 (12 bits total).
        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = HEADER_LEN + section_length;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len() - HEADER_LEN,
            });
        }

        // private_indicator: bit 6 of byte 1 (section_syntax_indicator is bit 7).
        let private_indicator = (bytes[1] & 0x40) != 0;

        // Extension header (bytes 3..13).
        let service_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        // byte 5: reserved(2) | version_number(5) | current_next_indicator(1)
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let transport_stream_id = u16::from_be_bytes([bytes[8], bytes[9]]);
        let original_network_id = u16::from_be_bytes([bytes[10], bytes[11]]);
        let prepend_strings_length = bytes[12];

        // Prepend-string block.
        let ps_start = HEADER_LEN + EXTENSION_LEN;
        let ps_end = ps_start + prepend_strings_length as usize;

        // Ensure prepend_strings block fits before the CRC.
        let payload_end = total - CRC_LEN;
        if ps_end > payload_end {
            return Err(Error::SectionLengthOverflow {
                declared: prepend_strings_length as usize,
                available: payload_end.saturating_sub(ps_start),
            });
        }

        let prepend_strings = &bytes[ps_start..ps_end];

        // Raw CRID entry loop: everything between prepend_strings and the CRC.
        let crid_entries = &bytes[ps_end..payload_end];

        Ok(Cit {
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

// ── Serialize ─────────────────────────────────────────────────────────────────

impl Serialize for Cit<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + EXTENSION_LEN + self.prepend_strings.len() + self.crid_entries.len() + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        // prepend_strings_length is an 8-bit wire field, derived from the slice.
        if self.prepend_strings.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.prepend_strings.len(),
                available: u8::MAX as usize,
            });
        }

        let section_length = (len - HEADER_LEN) as u16;

        // Byte 0: table_id.
        buf[0] = TABLE_ID;

        // Byte 1: section_syntax_indicator(1) | private_indicator(1) | reserved(2) | section_length[11:8](4).
        // section_syntax_indicator = 1 (long-form section per DVB convention).
        buf[1] = 0x80
            | (u8::from(self.private_indicator) << 6)
            | 0x30 // reserved bits set
            | ((section_length >> 8) as u8 & 0x0F);

        // Byte 2: section_length[7:0].
        buf[2] = (section_length & 0xFF) as u8;

        // Extension header.
        buf[3..5].copy_from_slice(&self.service_id.to_be_bytes());
        buf[5] = 0xC0 // reserved(2) = 11
            | ((self.version_number & 0x1F) << 1)
            | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8..10].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[10..12].copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[12] = self.prepend_strings.len() as u8;

        // Prepend strings.
        let ps_start = HEADER_LEN + EXTENSION_LEN;
        let ps_end = ps_start + self.prepend_strings.len();
        buf[ps_start..ps_end].copy_from_slice(self.prepend_strings);

        // CRID entries.
        let crid_end = ps_end + self.crid_entries.len();
        buf[ps_end..crid_end].copy_from_slice(self.crid_entries);

        // CRC-32: compute over everything up to (but not including) the CRC slot.
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crid_end]);
        buf[crid_end..len].copy_from_slice(&crc.to_be_bytes());

        Ok(len)
    }
}

// ── Table impl ────────────────────────────────────────────────────────────────

impl<'a> Table<'a> for Cit<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Cit<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "CONTENT_IDENTIFIER";
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a syntactically valid CIT section from its constituent fields.
    ///
    /// `prepend_strings` and `crid_entries` are raw byte slices; CRC is zeroed
    /// (matching the serializer convention).
    #[allow(clippy::too_many_arguments)]
    fn build_cit(
        service_id: u16,
        version: u8,
        current_next: bool,
        section_number: u8,
        last_section_number: u8,
        transport_stream_id: u16,
        original_network_id: u16,
        prepend_strings: &[u8],
        crid_entries: &[u8],
    ) -> Vec<u8> {
        let cit = Cit {
            private_indicator: false,
            service_id,
            version_number: version,
            current_next_indicator: current_next,
            section_number,
            last_section_number,
            transport_stream_id,
            original_network_id,
            prepend_strings,
            crid_entries,
        };
        let mut buf = vec![0u8; cit.serialized_len()];
        cit.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn parse_happy_path_no_crid_entries() {
        // A CIT section with no prepend strings and no CRID entries is the
        // minimal valid form; commonly seen on transponders that carry the CIT
        // structure but have no current programme mapping.
        let prepend = b"CRID://example.com\x00";
        let bytes = build_cit(0x1234, 3, true, 0, 0, 0x0064, 0x0002, prepend, &[]);
        let cit = Cit::parse(&bytes).unwrap();

        assert_eq!(cit.service_id, 0x1234);
        assert_eq!(cit.version_number, 3);
        assert!(cit.current_next_indicator);
        assert_eq!(cit.section_number, 0);
        assert_eq!(cit.last_section_number, 0);
        assert_eq!(cit.transport_stream_id, 0x0064);
        assert_eq!(cit.original_network_id, 0x0002);
        assert_eq!(cit.prepend_strings, prepend);
        assert_eq!(cit.crid_entries, &[] as &[u8]);
    }

    #[test]
    fn parse_happy_path_with_crid_entries() {
        // Two synthetic CRID entries:
        //   entry 0: crid_ref=0x0001, prepend_string_index=0x00, unique="ep1"
        //   entry 1: crid_ref=0x0002, prepend_string_index=0xFF (full CRID in unique), unique="crid://bbc.co.uk/EV-1"
        let prepend = b"crid://bbc.co.uk/\x00";
        let mut crid_entries: Vec<u8> = Vec::new();
        // Entry 0
        crid_entries.extend_from_slice(&0x0001u16.to_be_bytes()); // crid_ref
        crid_entries.push(0x00); // prepend_string_index
        let unique0 = b"ep1";
        crid_entries.push(unique0.len() as u8); // unique_string_length
        crid_entries.extend_from_slice(unique0);
        // Entry 1
        crid_entries.extend_from_slice(&0x0002u16.to_be_bytes());
        crid_entries.push(0xFF); // no prepend
        let unique1 = b"crid://bbc.co.uk/EV-1";
        crid_entries.push(unique1.len() as u8);
        crid_entries.extend_from_slice(unique1);

        let bytes = build_cit(
            0xABCD,
            7,
            true,
            1,
            3,
            0x01F4,
            0x0028,
            prepend,
            &crid_entries,
        );
        let cit = Cit::parse(&bytes).unwrap();

        assert_eq!(cit.service_id, 0xABCD);
        assert_eq!(cit.version_number, 7);
        assert_eq!(cit.section_number, 1);
        assert_eq!(cit.last_section_number, 3);
        assert_eq!(cit.transport_stream_id, 0x01F4);
        assert_eq!(cit.original_network_id, 0x0028);
        assert_eq!(cit.prepend_strings, prepend);
        assert_eq!(cit.crid_entries, crid_entries.as_slice());
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_cit(0x0001, 0, true, 0, 0, 0x0001, 0x0001, &[], &[]);
        bytes[0] = 0x40; // Not 0x77.
        assert!(matches!(
            Cit::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x40, .. }
        ));
    }

    #[test]
    fn parse_rejects_buffer_too_short() {
        // Hand-craft a buffer that is shorter than MIN_SECTION_LEN.
        let short = [TABLE_ID, 0x00];
        assert!(matches!(
            Cit::parse(&short).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_section_length_overflow() {
        let mut bytes = build_cit(0x0001, 0, true, 0, 0, 0x0001, 0x0001, &[], &[]);
        // Inflate the declared section_length to exceed actual buffer size.
        let fake_sl: u16 = (bytes.len() as u16) + 100 - HEADER_LEN as u16;
        bytes[1] = (bytes[1] & 0xF0) | ((fake_sl >> 8) as u8 & 0x0F);
        bytes[2] = (fake_sl & 0xFF) as u8;
        assert!(matches!(
            Cit::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let prepend = b"crid://example.com/\x00";
        let crid_entries = {
            let mut v: Vec<u8> = Vec::new();
            v.extend_from_slice(&0x0042u16.to_be_bytes());
            v.push(0x00);
            let unique = b"episode42";
            v.push(unique.len() as u8);
            v.extend_from_slice(unique);
            v
        };

        let original = Cit {
            private_indicator: true,
            service_id: 0x4321,
            version_number: 15,
            current_next_indicator: false,
            section_number: 2,
            last_section_number: 4,
            transport_stream_id: 0x03E8,
            original_network_id: 0x0050,
            prepend_strings: prepend,
            crid_entries: &crid_entries,
        };

        let mut buf = vec![0u8; original.serialized_len()];
        original.serialize_into(&mut buf).unwrap();
        let parsed = Cit::parse(&buf).unwrap();

        assert_eq!(parsed.private_indicator, original.private_indicator);
        assert_eq!(parsed.service_id, original.service_id);
        assert_eq!(parsed.version_number, original.version_number);
        assert_eq!(
            parsed.current_next_indicator,
            original.current_next_indicator
        );
        assert_eq!(parsed.section_number, original.section_number);
        assert_eq!(parsed.last_section_number, original.last_section_number);
        assert_eq!(parsed.transport_stream_id, original.transport_stream_id);
        assert_eq!(parsed.original_network_id, original.original_network_id);
        assert_eq!(parsed.prepend_strings, original.prepend_strings);
        assert_eq!(parsed.crid_entries, original.crid_entries);
    }

    #[test]
    fn serialize_rejects_output_buffer_too_small() {
        let cit = Cit {
            private_indicator: false,
            service_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            transport_stream_id: 0x0001,
            original_network_id: 0x0001,
            prepend_strings: &[],
            crid_entries: &[],
        };
        let mut buf = vec![0u8; 2]; // Far too small.
        assert!(matches!(
            cit.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }
}
