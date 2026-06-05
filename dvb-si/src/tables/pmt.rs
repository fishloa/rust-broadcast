//! Program Map Table — MPEG-2 ISO/IEC 13818-1 §2.4.4.8.
//!
//! PMT describes the elementary streams that make up one programme.
//! Carried on a per-programme PID signalled by the PAT, with table_id 0x02.
//! Descriptor parsing is out of scope for this commit — raw bytes only.

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// PMT table_id (ISO/IEC 13818-1 Table 2-30).
pub const TABLE_ID: u8 = 0x02;
/// PMT PIDs are programme-specific and signalled via PAT; 0x0000 is a
/// placeholder meaning "no well-known PID".
pub const PID: u16 = 0x0000;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const PCR_PID_LEN: usize = 2;
const PROG_INFO_LEN_BYTES: usize = 2;
const CRC_LEN: usize = 4;
const STREAM_HEADER_LEN: usize = 5;

/// One elementary stream entry in the PMT's ES loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct PmtStream<'a> {
    /// MPEG-2 stream_type byte (ISO/IEC 13818-1 Table 2-34).
    pub stream_type: u8,
    /// 13-bit elementary stream PID.
    pub elementary_pid: u16,
    /// Raw ES_info descriptor bytes; parsing lives in crate::descriptors.
    /// Elementary-stream descriptor loop. Serializes as the typed descriptor
    /// sequence; `.raw()` yields the wire bytes.
    pub es_info: DescriptorLoop<'a>,
}

/// Program Map Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Pmt<'a> {
    /// Programme number from the table_id_extension field.
    pub program_number: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// 13-bit PCR PID.
    pub pcr_pid: u16,
    /// Raw program_info descriptor bytes.
    /// Program-info descriptor loop. Serializes as the typed descriptor
    /// sequence; `.raw()` yields the wire bytes.
    pub program_info: DescriptorLoop<'a>,
    /// Elementary streams in wire order.
    pub streams: Vec<PmtStream<'a>>,
}

impl<'a> Parse<'a> for Pmt<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len =
            MIN_HEADER_LEN + EXTENSION_HEADER_LEN + PCR_PID_LEN + PROG_INFO_LEN_BYTES + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Pmt",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Pmt",
                expected: &[TABLE_ID],
            });
        }

        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = MIN_HEADER_LEN + section_length as usize;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: bytes.len() - MIN_HEADER_LEN,
            });
        }

        let program_number = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;

        let pcr_pid = (((bytes[8] & 0x1F) as u16) << 8) | bytes[9] as u16;
        let program_info_length = (((bytes[10] & 0x0F) as usize) << 8) | bytes[11] as usize;

        let prog_info_start =
            MIN_HEADER_LEN + EXTENSION_HEADER_LEN + PCR_PID_LEN + PROG_INFO_LEN_BYTES;
        let prog_info_end = prog_info_start + program_info_length;
        let stream_loop_end = total - CRC_LEN;
        if prog_info_end > stream_loop_end {
            return Err(Error::SectionLengthOverflow {
                declared: program_info_length,
                available: stream_loop_end - prog_info_start,
            });
        }
        let program_info = DescriptorLoop::new(&bytes[prog_info_start..prog_info_end]);

        let mut streams = Vec::new();
        let mut pos = prog_info_end;
        while pos + STREAM_HEADER_LEN <= stream_loop_end {
            let stream_type = bytes[pos];
            let elementary_pid = (((bytes[pos + 1] & 0x1F) as u16) << 8) | bytes[pos + 2] as u16;
            let es_info_length =
                (((bytes[pos + 3] & 0x0F) as usize) << 8) | bytes[pos + 4] as usize;
            let es_start = pos + STREAM_HEADER_LEN;
            let es_end = es_start + es_info_length;
            if es_end > stream_loop_end {
                return Err(Error::SectionLengthOverflow {
                    declared: es_info_length,
                    available: stream_loop_end - es_start,
                });
            }
            streams.push(PmtStream {
                stream_type,
                elementary_pid,
                es_info: DescriptorLoop::new(&bytes[es_start..es_end]),
            });
            pos = es_end;
        }

        Ok(Pmt {
            program_number,
            version_number,
            current_next_indicator,
            pcr_pid,
            program_info,
            streams,
        })
    }
}

impl Serialize for Pmt<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let streams_bytes: usize = self
            .streams
            .iter()
            .map(|s| STREAM_HEADER_LEN + s.es_info.len())
            .sum();
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + PCR_PID_LEN
            + PROG_INFO_LEN_BYTES
            + self.program_info.len()
            + streams_bytes
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

        let section_length: u16 = (len - MIN_HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&self.program_number.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = 0;
        buf[7] = 0;
        buf[8] = 0xE0 | ((self.pcr_pid >> 8) as u8 & 0x1F);
        buf[9] = (self.pcr_pid & 0xFF) as u8;
        let pil = self.program_info.len() as u16;
        buf[10] = 0xF0 | ((pil >> 8) as u8 & 0x0F);
        buf[11] = (pil & 0xFF) as u8;

        let prog_info_start =
            MIN_HEADER_LEN + EXTENSION_HEADER_LEN + PCR_PID_LEN + PROG_INFO_LEN_BYTES;
        buf[prog_info_start..prog_info_start + self.program_info.len()]
            .copy_from_slice(self.program_info.raw());

        let mut pos = prog_info_start + self.program_info.len();
        for stream in &self.streams {
            buf[pos] = stream.stream_type;
            buf[pos + 1] = 0xE0 | ((stream.elementary_pid >> 8) as u8 & 0x1F);
            buf[pos + 2] = (stream.elementary_pid & 0xFF) as u8;
            let esl = stream.es_info.len() as u16;
            buf[pos + 3] = 0xF0 | ((esl >> 8) as u8 & 0x0F);
            buf[pos + 4] = (esl & 0xFF) as u8;
            let es_start = pos + STREAM_HEADER_LEN;
            buf[es_start..es_start + stream.es_info.len()].copy_from_slice(stream.es_info.raw());
            pos = es_start + stream.es_info.len();
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for Pmt<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Pmt<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "PROGRAM_MAP";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a PMT section with given fields. Placeholder CRC.
    fn build_pmt(
        program_number: u16,
        version: u8,
        pcr_pid: u16,
        program_info: &[u8],
        streams: &[(u8, u16, Vec<u8>)],
    ) -> Vec<u8> {
        let streams_bytes: usize = streams
            .iter()
            .map(|(_, _, es)| STREAM_HEADER_LEN + es.len())
            .sum();
        let section_length: u16 = (EXTENSION_HEADER_LEN
            + PCR_PID_LEN
            + PROG_INFO_LEN_BYTES
            + program_info.len()
            + streams_bytes
            + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID);
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&program_number.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0);
        v.push(0);
        v.push(0xE0 | ((pcr_pid >> 8) as u8 & 0x1F));
        v.push((pcr_pid & 0xFF) as u8);
        v.push(0xF0 | ((program_info.len() >> 8) as u8 & 0x0F));
        v.push((program_info.len() & 0xFF) as u8);
        v.extend_from_slice(program_info);
        for (stype, pid, es) in streams {
            v.push(*stype);
            v.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
            v.push((pid & 0xFF) as u8);
            v.push(0xF0 | ((es.len() >> 8) as u8 & 0x0F));
            v.push((es.len() & 0xFF) as u8);
            v.extend_from_slice(es);
        }
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_extracts_pcr_pid_and_program_info() {
        let bytes = build_pmt(42, 5, 0x0100, &[0xAA, 0xBB], &[]);
        let pmt = Pmt::parse(&bytes).unwrap();
        assert_eq!(pmt.program_number, 42);
        assert_eq!(pmt.version_number, 5);
        assert!(pmt.current_next_indicator);
        assert_eq!(pmt.pcr_pid, 0x0100);
        assert_eq!(pmt.program_info.raw(), &[0xAA, 0xBB]);
        assert_eq!(pmt.streams.len(), 0);
    }

    #[test]
    fn parse_elementary_streams_and_es_info_slices() {
        let bytes = build_pmt(
            1,
            0,
            0x101,
            &[],
            &[(0x02, 0x102, vec![0x11, 0x22]), (0x1B, 0x103, vec![0x33])],
        );
        let pmt = Pmt::parse(&bytes).unwrap();
        assert_eq!(pmt.streams.len(), 2);
        assert_eq!(pmt.streams[0].stream_type, 0x02);
        assert_eq!(pmt.streams[0].elementary_pid, 0x102);
        assert_eq!(pmt.streams[0].es_info.raw(), &[0x11, 0x22]);
        assert_eq!(pmt.streams[1].stream_type, 0x1B);
        assert_eq!(pmt.streams[1].elementary_pid, 0x103);
        assert_eq!(pmt.streams[1].es_info.raw(), &[0x33]);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_pmt(1, 0, 0x100, &[], &[]);
        bytes[0] = 0x00;
        let err = Pmt::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x00, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Pmt::parse(&[0x02, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip_empty_program() {
        let pmt = Pmt {
            program_number: 1,
            version_number: 0,
            current_next_indicator: true,
            pcr_pid: 0x100,
            program_info: DescriptorLoop::new(&[]),
            streams: vec![],
        };
        let mut buf = vec![0u8; pmt.serialized_len()];
        pmt.serialize_into(&mut buf).unwrap();
        let re = Pmt::parse(&buf).unwrap();
        assert_eq!(pmt, re);
    }

    #[test]
    fn serialize_round_trip_with_streams_and_descriptors() {
        let prog_info: [u8; 3] = [0x09, 0x01, 0xFF];
        let es1: [u8; 4] = [0x52, 0x02, 0xAA, 0xBB];
        let es2: [u8; 2] = [0x0A, 0x00];
        let pmt = Pmt {
            program_number: 0xABCD,
            version_number: 7,
            current_next_indicator: true,
            pcr_pid: 0x1F0,
            program_info: DescriptorLoop::new(&prog_info),
            streams: vec![
                PmtStream {
                    stream_type: 0x02,
                    elementary_pid: 0x100,
                    es_info: DescriptorLoop::new(&es1),
                },
                PmtStream {
                    stream_type: 0x03,
                    elementary_pid: 0x101,
                    es_info: DescriptorLoop::new(&es2),
                },
                PmtStream {
                    stream_type: 0x1B,
                    elementary_pid: 0x102,
                    es_info: DescriptorLoop::new(&[]),
                },
            ],
        };
        let mut buf = vec![0u8; pmt.serialized_len()];
        pmt.serialize_into(&mut buf).unwrap();
        let re = Pmt::parse(&buf).unwrap();
        assert_eq!(pmt, re);
    }

    #[test]
    fn zero_elementary_streams_is_valid() {
        let bytes = build_pmt(99, 0, 0x0100, &[], &[]);
        let pmt = Pmt::parse(&bytes).unwrap();
        assert_eq!(pmt.streams.len(), 0);
    }

    #[test]
    fn parse_preserves_raw_program_info_bytes() {
        let pi = vec![0x09, 0x04, 0x01, 0x02, 0x03, 0x04];
        let bytes = build_pmt(1, 0, 0x100, &pi, &[]);
        let pmt = Pmt::parse(&bytes).unwrap();
        assert_eq!(pmt.program_info.raw(), &pi[..]);
    }
}
