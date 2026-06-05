//! Related Content Table — ETSI TS 102 323 v1.4.1 §10.4.
//!
//! Signals links to related material for a service. Carried in the ES whose
//! PID is named by a `related_content_descriptor` in that service's PMT
//! (stream_type 0x05, private sections). There is no fixed PID.
//!
//! Structure (§10.4.2 Table 109):
//!   table_id(8) | section_syntax_indicator(1) | table_id_extension_flag(1) |
//!   reserved(2) | section_length(12) | service_id(16) | reserved(2) |
//!   version_number(5) | current_next_indicator(1) | section_number(8) |
//!   last_section_number(8) | year_offset(16) | link_count(8) |
//!   for j<link_count { reserved(4) | link_info_length(12) | link_info() } |
//!   reserved_future_use(4) | descriptor_loop_length(12) | descriptors |
//!   CRC_32(32)

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for Related Content Table.
pub const TABLE_ID: u8 = 0x76;

/// Well-known PID on which RCT is carried.
///
/// RCT is signalled — its ES PID is named by a `related_content_descriptor`
/// in the service PMT. There is no fixed broadcast PID. This constant is
/// `0x0000` per the `Table` trait contract for tables without a fixed PID.
pub const PID: u16 = 0x0000;

// ── Length constants ────────────────────────────────────────────────────────

/// Bytes 0-2: table_id + the section_syntax/flags/section_length word.
const MIN_HEADER_LEN: usize = 3;

/// Bytes 3-7: service_id(2) + version/cni byte(1) + section_number(1) +
/// last_section_number(1).
const EXTENSION_HEADER_LEN: usize = 5;

/// Bytes after extension header: year_offset(2) + link_count(1) = 3 bytes.
const POST_EXT_FIXED_LEN: usize = 3;

/// Per-link header: reserved(4) + link_info_length(12) = 2 bytes.
const LINK_ENTRY_HEADER_LEN: usize = 2;

/// Descriptor loop length field: reserved_future_use(4) + descriptor_loop_length(12) = 2 bytes.
const DESC_LOOP_LEN_FIELD: usize = 2;

/// CRC-32 trailer.
const CRC_LEN: usize = 4;

/// Minimum parseable section length (no links, no descriptors).
const MIN_SECTION_LEN: usize =
    MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN + DESC_LOOP_LEN_FIELD + CRC_LEN;

// ── Structs ─────────────────────────────────────────────────────────────────

/// Related Content Table (ETSI TS 102 323 v1.4.1 §10.4.2).
///
/// The `link_info_loop` field holds the raw bytes of the entire
/// `for (j=0; j<link_count; j++)` block (all link entries concatenated).
/// The `descriptors` field holds the raw bytes of the trailing descriptor loop.
/// Neither is parsed further; the integrator walks them using the spec tables.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Rct<'a> {
    /// `table_id_extension_flag` (bit 6 of byte 1).
    ///
    /// `false`: `service_id` identifies the service this sub-table belongs to.
    /// `true`: all sections relate to a single service; `service_id` is ignored.
    pub table_id_extension_flag: bool,

    /// `service_id` — table_id_extension field (bytes 3-4).
    pub service_id: u16,

    /// 5-bit `version_number` (bits 5-1 of byte 5).
    pub version_number: u8,

    /// `current_next_indicator` (bit 0 of byte 5).
    pub current_next_indicator: bool,

    /// `section_number` (byte 6).
    pub section_number: u8,

    /// `last_section_number` (byte 7).
    pub last_section_number: u8,

    /// `year_offset` — reference year (bytes 8-9).
    ///
    /// Binary encoding, e.g. `0x07D3` = 2003. Date values inside the section
    /// are relative to this year.
    pub year_offset: u16,

    /// Number of link entries (`link_count`, byte 10).
    pub link_count: u8,

    /// Raw bytes of the entire link_info loop
    /// (`for j=0; j<link_count` block from §10.4.2 Table 109).
    ///
    /// Each entry begins with a 2-byte header: `reserved(4) | link_info_length(12)`.
    /// The following `link_info_length` bytes contain the link_info() payload
    /// (§10.4.3 Table 110). The integrator is responsible for further parsing.
    pub link_info_loop: &'a [u8],

    /// Trailing descriptor loop
    /// (`for k=0; k<descriptor_loop_length` from §10.4.2 Table 109).
    /// Serializes as the typed descriptor sequence; `.raw()` yields the bytes.
    pub descriptors: DescriptorLoop<'a>,
}

// ── Parse ────────────────────────────────────────────────────────────────────

impl<'a> Parse<'a> for Rct<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "Rct",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Rct",
                expected: &[TABLE_ID],
            });
        }

        // Byte 1: section_syntax_indicator(1) | table_id_extension_flag(1) | reserved(2) |
        //         section_length_hi(4)
        // Byte 2: section_length_lo(8)
        let table_id_extension_flag = (bytes[1] & 0x40) != 0;
        let section_length = (((bytes[1] & 0x0F) as u16) << 8) | bytes[2] as u16;
        let total = MIN_HEADER_LEN + section_length as usize;

        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: bytes.len() - MIN_HEADER_LEN,
            });
        }

        // Byte 3-4: service_id
        let service_id = u16::from_be_bytes([bytes[3], bytes[4]]);

        // Byte 5: reserved(2) | version_number(5) | current_next_indicator(1)
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;

        // Byte 6: section_number
        // Byte 7: last_section_number
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        // Byte 8-9: year_offset
        let year_offset = u16::from_be_bytes([bytes[8], bytes[9]]);

        // Byte 10: link_count
        let link_count = bytes[10];

        // Walk the link_info loop to find its total byte span.
        // Each entry: 2-byte header (reserved(4)|link_info_length(12)) + link_info_length bytes.
        let payload_end = total - CRC_LEN; // byte index just past last payload byte
        let mut pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN; // byte 11
        let link_loop_start = pos;

        for j in 0..link_count {
            if pos + LINK_ENTRY_HEADER_LEN > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + LINK_ENTRY_HEADER_LEN,
                    have: payload_end,
                    what: "Rct link_entry header",
                });
            }
            let link_info_length = (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
            let entry_end = pos + LINK_ENTRY_HEADER_LEN + link_info_length;
            if entry_end > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: link_info_length,
                    available: payload_end.saturating_sub(pos + LINK_ENTRY_HEADER_LEN),
                });
            }
            let _ = j; // used only for the error message above
            pos = entry_end;
        }

        let link_info_loop = &bytes[link_loop_start..pos];

        // Descriptor loop length field: reserved_future_use(4) | descriptor_loop_length(12)
        if pos + DESC_LOOP_LEN_FIELD > payload_end {
            return Err(Error::BufferTooShort {
                need: pos + DESC_LOOP_LEN_FIELD,
                have: payload_end,
                what: "Rct descriptor_loop_length field",
            });
        }
        let descriptor_loop_length =
            (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
        let desc_start = pos + DESC_LOOP_LEN_FIELD;
        let desc_end = desc_start + descriptor_loop_length;

        if desc_end > payload_end {
            return Err(Error::SectionLengthOverflow {
                declared: descriptor_loop_length,
                available: payload_end.saturating_sub(desc_start),
            });
        }

        let descriptors = DescriptorLoop::new(&bytes[desc_start..desc_end]);

        Ok(Rct {
            table_id_extension_flag,
            service_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            year_offset,
            link_count,
            link_info_loop,
            descriptors,
        })
    }
}

// ── Serialize ────────────────────────────────────────────────────────────────

impl Serialize for Rct<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + POST_EXT_FIXED_LEN
            + self.link_info_loop.len()
            + DESC_LOOP_LEN_FIELD
            + self.descriptors.len()
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

        let section_length = (len - MIN_HEADER_LEN) as u16;

        // Byte 0: table_id
        buf[0] = TABLE_ID;

        // Byte 1: section_syntax_indicator(1)=1 | table_id_extension_flag(1) |
        //         reserved(2)=11 | section_length_hi(4)
        let tief_bit: u8 = if self.table_id_extension_flag {
            0x40
        } else {
            0x00
        };
        buf[1] = 0x80 | tief_bit | 0x30 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        // Bytes 3-4: service_id
        buf[3..5].copy_from_slice(&self.service_id.to_be_bytes());

        // Byte 5: reserved(2)=11 | version_number(5) | current_next_indicator(1)
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);

        // Bytes 6-7: section_number, last_section_number
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        // Bytes 8-9: year_offset
        buf[8..10].copy_from_slice(&self.year_offset.to_be_bytes());

        // Byte 10: link_count
        buf[10] = self.link_count;

        // Link info loop (raw bytes, already framed with per-entry headers)
        let loop_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN;
        let loop_end = loop_start + self.link_info_loop.len();
        buf[loop_start..loop_end].copy_from_slice(self.link_info_loop);

        // Descriptor loop length field: reserved_future_use(4)=1111 | descriptor_loop_length(12)
        let dll = self.descriptors.len() as u16;
        buf[loop_end] = 0xF0 | ((dll >> 8) as u8 & 0x0F);
        buf[loop_end + 1] = (dll & 0xFF) as u8;

        let desc_start = loop_end + DESC_LOOP_LEN_FIELD;
        let desc_end = desc_start + self.descriptors.len();
        buf[desc_start..desc_end].copy_from_slice(self.descriptors.raw());

        // CRC-32: compute over everything up to (but not including) the CRC slot.
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..desc_end]);
        buf[desc_end..desc_end + CRC_LEN].copy_from_slice(&crc.to_be_bytes());

        Ok(len)
    }
}

// ── Table impl ────────────────────────────────────────────────────────────────

impl<'a> Table<'a> for Rct<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Rct<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "RELATED_CONTENT";
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal RCT section byte vector suitable for parsing.
    ///
    /// `link_info_loop_bytes` must already be formatted as the raw wire bytes
    /// for the entire link loop (i.e. `link_count` entries each preceded by
    /// their 2-byte `reserved|link_info_length` header).
    #[allow(clippy::too_many_arguments)]
    fn build_rct(
        service_id: u16,
        version: u8,
        current_next: bool,
        section_number: u8,
        last_section_number: u8,
        year_offset: u16,
        link_count: u8,
        link_info_loop_bytes: &[u8],
        descriptors: &[u8],
    ) -> Vec<u8> {
        let rct = Rct {
            table_id_extension_flag: false,
            service_id,
            version_number: version,
            current_next_indicator: current_next,
            section_number,
            last_section_number,
            year_offset,
            link_count,
            link_info_loop: link_info_loop_bytes,
            descriptors: DescriptorLoop::new(descriptors),
        };
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn parse_happy_path_no_links_no_descriptors() {
        // service_id=0x0064 (100), version=3, current=true, sec 0/0,
        // year_offset=2003 (0x07D3), link_count=0
        let bytes = build_rct(0x0064, 3, true, 0, 0, 0x07D3, 0, &[], &[]);
        let rct = Rct::parse(&bytes).unwrap();

        assert!(!rct.table_id_extension_flag);
        assert_eq!(rct.service_id, 0x0064);
        assert_eq!(rct.version_number, 3);
        assert!(rct.current_next_indicator);
        assert_eq!(rct.section_number, 0);
        assert_eq!(rct.last_section_number, 0);
        assert_eq!(rct.year_offset, 0x07D3);
        assert_eq!(rct.link_count, 0);
        assert_eq!(rct.link_info_loop, &[] as &[u8]);
        assert_eq!(rct.descriptors.raw(), &[] as &[u8]);
    }

    #[test]
    fn parse_happy_path_with_one_link_and_descriptor() {
        // Construct one link_info entry of 4 bytes of payload.
        // Header: reserved(4)=0xF | link_info_length=4 → bytes [0xF0, 0x04]
        // Payload: arbitrary 4 bytes.
        let link_payload: &[u8] = &[0xF0, 0x04, 0xAB, 0xCD, 0xEF, 0x01];
        // One private descriptor (tag=0x80, length=2, data=[0x01, 0x02]).
        let desc: &[u8] = &[0x80, 0x02, 0x01, 0x02];

        let bytes = build_rct(0x1234, 7, true, 1, 3, 2003, 1, link_payload, desc);
        let rct = Rct::parse(&bytes).unwrap();

        assert_eq!(rct.service_id, 0x1234);
        assert_eq!(rct.version_number, 7);
        assert_eq!(rct.link_count, 1);
        assert_eq!(rct.link_info_loop, link_payload);
        assert_eq!(rct.descriptors.raw(), desc);
        assert_eq!(rct.section_number, 1);
        assert_eq!(rct.last_section_number, 3);
        assert_eq!(rct.year_offset, 2003);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_rct(0x0001, 0, true, 0, 0, 2024, 0, &[], &[]);
        bytes[0] = 0x4A; // BAT table_id
        let err = Rct::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x4A, .. }
        ));
    }

    #[test]
    fn parse_rejects_buffer_too_short() {
        // Less than MIN_SECTION_LEN bytes.
        let err = Rct::parse(&[0x76, 0x80, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let link_loop: &[u8] = &[0xF0, 0x03, 0x11, 0x22, 0x33];
        let desc: &[u8] = &[0x58, 0x00]; // local_time_offset_descriptor, length 0

        let rct = Rct {
            table_id_extension_flag: true,
            service_id: 0xABCD,
            version_number: 15,
            current_next_indicator: false,
            section_number: 2,
            last_section_number: 5,
            year_offset: 2024,
            link_count: 1,
            link_info_loop: link_loop,
            descriptors: DescriptorLoop::new(desc),
        };

        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        let parsed = Rct::parse(&buf).unwrap();

        assert_eq!(parsed.table_id_extension_flag, rct.table_id_extension_flag);
        assert_eq!(parsed.service_id, rct.service_id);
        assert_eq!(parsed.version_number, rct.version_number);
        assert_eq!(parsed.current_next_indicator, rct.current_next_indicator);
        assert_eq!(parsed.section_number, rct.section_number);
        assert_eq!(parsed.last_section_number, rct.last_section_number);
        assert_eq!(parsed.year_offset, rct.year_offset);
        assert_eq!(parsed.link_count, rct.link_count);
        assert_eq!(parsed.link_info_loop, rct.link_info_loop);
        assert_eq!(parsed.descriptors, rct.descriptors);
    }
}
