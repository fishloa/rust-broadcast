//! Program Association Table — MPEG-2 ISO/IEC 13818-1 §2.4.4.3.
//!
//! PAT is carried on PID 0x0000 with table_id 0x00. It maps
//! program_number values to the PID on which their PMT is carried.
//! program_number 0x0000 is special — its PID is the NIT PID.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// PAT table_id (ISO/IEC 13818-1 Table 2-30).
pub const TABLE_ID: u8 = 0x00;
/// PAT well-known PID.
pub const PID: u16 = 0x0000;
/// program_number value that carries the NIT PID rather than a PMT PID.
pub const PROGRAM_NUMBER_NIT: u16 = 0x0000;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const CRC_LEN: usize = 4;
const MIN_SECTION_LEN: usize = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN;
const ENTRY_LEN: usize = 4;

/// One entry in the PAT program loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PatEntry {
    /// program_number. 0x0000 means "next PID is the NIT PID".
    pub program_number: u16,
    /// PMT PID (or NIT PID when program_number == 0).
    pub pid: u16,
}

/// Program Association Table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PatSection {
    /// transport_stream_id from the section header.
    pub transport_stream_id: u16,
    /// 5-bit version_number from the section header.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// Program entries in wire order.
    pub entries: Vec<PatEntry>,
}

impl<'a> Parse<'a> for PatSection {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN,
                have: bytes.len(),
                what: "PatSection",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "PatSection",
                expected: &[TABLE_ID],
            });
        }

        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            section_length as usize,
            MIN_SECTION_LEN,
        )?;

        let transport_stream_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let end = total - CRC_LEN;
        let mut entries = Vec::new();
        let mut pos = 8;
        while pos < end {
            if pos + ENTRY_LEN > end {
                break;
            }
            let chunk = &bytes[pos..pos + ENTRY_LEN];
            let program_number = u16::from_be_bytes([chunk[0], chunk[1]]);
            let pid = (((chunk[2] & 0x1F) as u16) << 8) | chunk[3] as u16;
            entries.push(PatEntry {
                program_number,
                pid,
            });
            pos += ENTRY_LEN;
        }

        Ok(PatSection {
            transport_stream_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            entries,
        })
    }
}

impl Serialize for PatSection {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN + EXTENSION_HEADER_LEN + self.entries.len() * ENTRY_LEN + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let section_length: u16 =
            (EXTENSION_HEADER_LEN + self.entries.len() * ENTRY_LEN + CRC_LEN) as u16;

        buf[0] = TABLE_ID;
        buf[1] = super::SECTION_B1_FLAGS_PSI | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        let mut pos = 8;
        for entry in &self.entries {
            buf[pos..pos + 2].copy_from_slice(&entry.program_number.to_be_bytes());
            buf[pos + 2] = 0xE0 | ((entry.pid >> 8) as u8 & 0x1F);
            buf[pos + 3] = (entry.pid & 0xFF) as u8;
            pos += ENTRY_LEN;
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());

        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for PatSection {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "PROGRAM_ASSOCIATION";
}

impl PatSection {
    /// Program entries excluding the NIT entry.
    pub fn programmes(&self) -> impl Iterator<Item = &PatEntry> {
        self.entries
            .iter()
            .filter(|e| e.program_number != PROGRAM_NUMBER_NIT)
    }

    /// NIT PID if this PAT carries an entry with program_number == 0.
    pub fn nit_pid(&self) -> Option<u16> {
        self.entries
            .iter()
            .find(|e| e.program_number == PROGRAM_NUMBER_NIT)
            .map(|e| e.pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a PAT section with the given entries and a placeholder CRC.
    fn build_pat(tsid: u16, version: u8, entries: &[(u16, u16)]) -> Vec<u8> {
        let section_length: u16 =
            (EXTENSION_HEADER_LEN + entries.len() * ENTRY_LEN + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID);
        v.push(super::super::SECTION_B1_FLAGS_PSI | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&tsid.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01); // version, cni=1
        v.push(0x00); // section_number
        v.push(0x00); // last_section_number
        for &(pn, pid) in entries {
            v.extend_from_slice(&pn.to_be_bytes());
            v.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
            v.push((pid & 0xFF) as u8);
        }
        v.extend_from_slice(&[0, 0, 0, 0]); // placeholder CRC
        v
    }

    #[test]
    fn parse_empty_pat_zero_programs() {
        let bytes = build_pat(0x1234, 5, &[]);
        let pat = PatSection::parse(&bytes).expect("parse");
        assert_eq!(pat.transport_stream_id, 0x1234);
        assert_eq!(pat.version_number, 5);
        assert!(pat.current_next_indicator);
        assert_eq!(pat.section_number, 0);
        assert_eq!(pat.last_section_number, 0);
        assert_eq!(pat.entries.len(), 0);
    }

    #[test]
    fn parse_single_program_extracts_pmt_pid() {
        let bytes = build_pat(1, 0, &[(42, 0x1234)]);
        let pat = PatSection::parse(&bytes).unwrap();
        assert_eq!(pat.entries.len(), 1);
        assert_eq!(pat.entries[0].program_number, 42);
        assert_eq!(pat.entries[0].pid, 0x1234);
    }

    #[test]
    fn parse_many_programs_preserves_order() {
        let entries: Vec<(u16, u16)> = (1..=10).map(|i| (i, 0x1000 + i)).collect();
        let bytes = build_pat(1, 0, &entries);
        let pat = PatSection::parse(&bytes).unwrap();
        assert_eq!(pat.entries.len(), 10);
        for (i, e) in pat.entries.iter().enumerate() {
            assert_eq!(e.program_number, (i + 1) as u16);
            assert_eq!(e.pid, 0x1000 + (i + 1) as u16);
        }
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_pat(1, 0, &[]);
        bytes[0] = 0x02; // PMT table_id
        let err = PatSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x02, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = PatSection::parse(&[0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip_empty() {
        let pat = PatSection {
            transport_stream_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries: vec![],
        };
        let mut buf = vec![0u8; pat.serialized_len()];
        pat.serialize_into(&mut buf).expect("serialize");
        let reparsed = PatSection::parse(&buf).expect("reparse");
        assert_eq!(pat, reparsed);
    }

    #[test]
    fn serialize_round_trip_many_programs() {
        let entries: Vec<PatEntry> = (1..=5)
            .map(|i| PatEntry {
                program_number: i,
                pid: 0x1000 + i,
            })
            .collect();
        let pat = PatSection {
            transport_stream_id: 0xABCD,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries,
        };
        let mut buf = vec![0u8; pat.serialized_len()];
        pat.serialize_into(&mut buf).unwrap();
        let reparsed = PatSection::parse(&buf).unwrap();
        assert_eq!(pat, reparsed);
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID;
        buf[1] = 0xB0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            PatSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn network_pid_entry_identified_by_program_number_0() {
        let bytes = build_pat(1, 0, &[(0, 0x0010), (1, 0x0100)]);
        let pat = PatSection::parse(&bytes).unwrap();
        assert_eq!(pat.nit_pid(), Some(0x0010));
        assert_eq!(pat.programmes().count(), 1);
        assert_eq!(pat.programmes().next().unwrap().program_number, 1);
    }
}
