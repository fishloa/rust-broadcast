//! Program Stream Map — ISO/IEC 13818-1 §2.5.4, Table 2-41.
//!
//! The PSM provides a mapping from `stream_type`/`elementary_stream_id`
//! to descriptors, tied to the Program Stream with a CRC-32 trailer.
//! It uses start-code prefix `0x000001` + `map_stream_id` `0xBC`.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use dvb_common::{crc32_mpeg2, Parse, Serialize};

/// `packet_start_code_prefix` — `0x000001`.
pub const PACKET_START_CODE_PREFIX: u32 = 0x00_0001;
/// `map_stream_id` — `0xBC`, combined with the prefix forms the PSM start code.
pub const MAP_STREAM_ID: u8 = 0xBC;
/// Combined PSM start code: `0x000001BC`.
#[allow(dead_code)]
pub const PSM_START_CODE: u32 = (PACKET_START_CODE_PREFIX << 8) | MAP_STREAM_ID as u32;

/// Bytes before the map body: prefix(3) + map_stream_id(1) + psm_length(2) = 6.
const PREFIX_LEN: usize = 6;
/// Total fixed header bytes: PREFIX_LEN + flags(1) + reserved(1) + prog_info_len(2) + es_map_len(2) = 13.
#[allow(dead_code)]
const HEADER_LEN: usize = 13;

/// An elementary stream descriptor entry with optional stream_id_extension.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EsMapEntry<'a> {
    /// `stream_type` per ISO/IEC 13818-1 Table 2-29.
    pub stream_type: u8,
    /// `elementary_stream_id`.
    pub elementary_stream_id: u8,
    /// If `elementary_stream_id == 0xFD` and `single_extension_stream_flag == 0`,
    /// this holds the `elementary_stream_id_extension`. Otherwise `None`.
    pub stream_id_extension: Option<u8>,
    /// Descriptor bytes for this elementary stream.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub descriptors: &'a [u8],
}

// Owned version for building/serializing
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedEsMapEntry {
    pub stream_type: u8,
    pub elementary_stream_id: u8,
    pub stream_id_extension: Option<u8>,
    pub descriptors: Vec<u8>,
}

/// A parsed Program Stream Map.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProgramStreamMap<'a> {
    /// `current_next_indicator` (1 bit).
    pub current_next_indicator: bool,
    /// `single_extension_stream_flag` (1 bit).
    pub single_extension_stream_flag: bool,
    /// `program_stream_map_version` (5 bits).
    pub version: u8,
    /// Descriptors for the program stream itself.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub program_stream_info: &'a [u8],
    /// Per-elementary-stream entries.
    pub elementary_stream_map: Vec<EsMapEntry<'a>>,
    /// The raw CRC-32 value in the trailer (validated on parse).
    pub crc: u32,
}

impl<'a> Parse<'a> for ProgramStreamMap<'a> {
    type Error = Error;

    fn parse(b: &'a [u8]) -> Result<Self> {
        if b.len() < PREFIX_LEN + 2 + 4 {
            // prefix + psm_length(2) + CRC(4) = minimum
            return Err(Error::BufferTooShort {
                need: PREFIX_LEN + 2 + 4,
                have: b.len(),
                what: "program_stream_map",
            });
        }

        // packet_start_code_prefix (3 bytes) + map_stream_id (1 byte)
        let start = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        if start != PACKET_START_CODE_PREFIX {
            return Err(Error::BadMapStreamId(b[3]));
        }
        if b[3] != MAP_STREAM_ID {
            return Err(Error::BadMapStreamId(b[3]));
        }

        let map_length = u16::from_be_bytes([b[4], b[5]]) as usize;
        let crc_end = PREFIX_LEN + map_length + 4;
        if b.len() < crc_end {
            return Err(Error::MapLengthOverflow {
                map_length,
                available: b.len().saturating_sub(PREFIX_LEN),
            });
        }

        // Validate CRC before trusting the parsed content
        let crc_data_end = crc_end - 4;
        let crc_data = &b[0..crc_data_end];
        let stored_crc = u32::from_be_bytes([
            b[crc_data_end],
            b[crc_data_end + 1],
            b[crc_data_end + 2],
            b[crc_data_end + 3],
        ]);
        let computed_crc = crc32_mpeg2::compute(crc_data);
        if computed_crc != stored_crc {
            return Err(Error::BadCrc {
                computed: computed_crc,
                stored: stored_crc,
            });
        }

        // flags byte: current_next(1) | single_extension(1) | reserved(1) | version(5)
        let flags = b[6];
        let current_next_indicator = flags & 0x80 != 0;
        let single_extension_stream_flag = flags & 0x40 != 0;
        let version = flags & 0x1F;

        // reserved byte (7 bits reserved + marker_bit)
        if b[7] & 0x01 == 0 {
            return Err(Error::BadMarker("program_stream_map marker_bit"));
        }

        let program_stream_info_length = u16::from_be_bytes([b[8], b[9]]) as usize;
        let elementary_stream_map_length = u16::from_be_bytes([b[10], b[11]]) as usize;

        let info_start = PREFIX_LEN + 4; // after flags(1)+reserved(1)+prog_info_len(2)
        let info_end = info_start + program_stream_info_length;
        // es_map_len field is 2 bytes before es data
        let es_start = info_end + 2; // after elementary_stream_map_length(2)
        let es_end = es_start + elementary_stream_map_length;

        if crc_data_end < es_end {
            return Err(Error::MapLengthOverflow {
                map_length,
                available: b.len().saturating_sub(PREFIX_LEN),
            });
        }

        let program_stream_info = &b[info_start..info_end];

        // Parse elementary stream loop
        let es_data = &b[es_start..es_end];
        let elementary_stream_map = parse_es_loop(es_data, single_extension_stream_flag)?;

        Ok(ProgramStreamMap {
            current_next_indicator,
            single_extension_stream_flag,
            version,
            program_stream_info,
            elementary_stream_map,
            crc: stored_crc,
        })
    }
}

fn parse_es_loop<'a>(data: &'a [u8], single_flag: bool) -> Result<Vec<EsMapEntry<'a>>> {
    let mut entries = Vec::new();
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let stream_type = data[pos];
        let elementary_stream_id = data[pos + 1];
        let es_info_length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        let entry_hdr_end = pos + 4;
        let entry_end = entry_hdr_end + es_info_length;
        if entry_end > data.len() {
            return Err(Error::BufferTooShort {
                need: entry_end,
                have: data.len(),
                what: "elementary_stream_map entry descriptors",
            });
        }

        let (stream_id_extension, descriptors) = if elementary_stream_id == 0xFD && !single_flag {
            // Extension form with pseudo descriptor
            if es_info_length < 3 {
                return Err(Error::BufferTooShort {
                    need: 3,
                    have: es_info_length,
                    what: "pseudo descriptor for stream_id_extension",
                });
            }
            // pseudo_descriptor_tag + pseudo_descriptor_length + marker+extension
            if data[entry_hdr_end + 2] & 0x80 == 0 {
                return Err(Error::BadMarker("elementary_stream_id_extension marker"));
            }
            let ext = data[entry_hdr_end + 2] & 0x7F;
            let desc = &data[entry_hdr_end + 3..entry_end];
            (Some(ext), desc)
        } else {
            let desc = &data[entry_hdr_end..entry_end];
            (None, desc)
        };

        entries.push(EsMapEntry {
            stream_type,
            elementary_stream_id,
            stream_id_extension,
            descriptors,
        });

        pos = entry_end;
    }
    Ok(entries)
}

fn serialize_es_loop(entries: &[OwnedEsMapEntry]) -> Vec<u8> {
    let mut buf = Vec::new();
    for e in entries {
        buf.push(e.stream_type);
        buf.push(e.elementary_stream_id);

        let desc_len = if e.stream_id_extension.is_some() {
            // pseudo descriptor: tag(1) + len(1) + marker+ext(1) + descriptors
            3 + e.descriptors.len()
        } else {
            e.descriptors.len()
        };

        buf.extend_from_slice(&(desc_len as u16).to_be_bytes());

        if let Some(ext) = e.stream_id_extension {
            buf.push(0x00); // pseudo_descriptor_tag (any value)
            buf.push(1 + e.descriptors.len() as u8); // pseudo_descriptor_length
            buf.push(0x80 | (ext & 0x7F)); // marker + extension
        }

        buf.extend_from_slice(&e.descriptors);
    }
    buf
}

impl Serialize for ProgramStreamMap<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let prog_info_len = self.program_stream_info.len();
        let es_loop_len: usize = self
            .elementary_stream_map
            .iter()
            .map(|e| {
                4 + if e.stream_id_extension.is_some() {
                    3 + e.descriptors.len()
                } else {
                    e.descriptors.len()
                }
            })
            .sum();
        // PREFIX_LEN(6) + map_body_len + CRC(4)
        6 + 6 + prog_info_len + es_loop_len + 4
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: buf.len(),
                what: "program_stream_map serialize output",
            });
        }

        // packet_start_code_prefix (3 bytes)
        buf[0..3].copy_from_slice(&PACKET_START_CODE_PREFIX.to_be_bytes()[1..]);
        // map_stream_id
        buf[3] = MAP_STREAM_ID;

        let prog_info_len = self.program_stream_info.len();
        let es_loop_data: Vec<u8> = serialize_es_loop(
            &self
                .elementary_stream_map
                .iter()
                .map(|e| OwnedEsMapEntry {
                    stream_type: e.stream_type,
                    elementary_stream_id: e.elementary_stream_id,
                    stream_id_extension: e.stream_id_extension,
                    descriptors: e.descriptors.to_vec(),
                })
                .collect::<Vec<_>>(),
        );
        let es_loop_len = es_loop_data.len();

        // map_length = flags(1) + reserved(1) + prog_info_len(2) + es_loop_len(2) + prog_info + es_loop
        let map_length = 6 + prog_info_len + es_loop_len;
        buf[4..6].copy_from_slice(&(map_length as u16).to_be_bytes());

        // flags: current_next(1) + single_extension(1) + reserved(1) + version(5)
        buf[6] = (u8::from(self.current_next_indicator) << 7)
            | (u8::from(self.single_extension_stream_flag) << 6)
            | (self.version & 0x1F);

        // reserved(7) + marker_bit(1)
        buf[7] = 0x7F | 0x01;

        // program_stream_info_length
        buf[8..10].copy_from_slice(&(prog_info_len as u16).to_be_bytes());
        // elementary_stream_map_length
        buf[10..12].copy_from_slice(&(es_loop_len as u16).to_be_bytes());

        // program_stream_info descriptors
        buf[12..12 + prog_info_len].copy_from_slice(self.program_stream_info);

        // elementary stream loop
        let es_start = 12 + prog_info_len;
        buf[es_start..es_start + es_loop_len].copy_from_slice(&es_loop_data);

        // CRC-32 over everything before it
        let crc_offset = es_start + es_loop_len;
        let crc = crc32_mpeg2::compute(&buf[0..crc_offset]);
        buf[crc_offset..crc_offset + 4].copy_from_slice(&crc.to_be_bytes());

        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// Build a valid PSM programmatically and verify round-trip.
    #[test]
    fn psm_build_and_round_trip() {
        let entries = vec![EsMapEntry {
            stream_type: 0x02, // MPEG-2 video
            elementary_stream_id: 0xE0,
            stream_id_extension: None,
            descriptors: &[0x0A, 0x04, b'H', b'E', b'L', b'L'], // registration descriptor
        }];

        let psm = ProgramStreamMap {
            current_next_indicator: true,
            single_extension_stream_flag: false,
            version: 3,
            program_stream_info: &[],
            elementary_stream_map: entries,
            crc: 0, // will be overwritten by serialize
        };

        let mut buf = vec![0u8; psm.serialized_len()];
        psm.serialize_into(&mut buf).unwrap();

        // Parse back
        let parsed = ProgramStreamMap::parse(&buf).unwrap();
        assert!(parsed.current_next_indicator);
        assert!(!parsed.single_extension_stream_flag);
        assert_eq!(parsed.version, 3);
        assert!(parsed.program_stream_info.is_empty());
        assert_eq!(parsed.elementary_stream_map.len(), 1);
        assert_eq!(parsed.elementary_stream_map[0].stream_type, 0x02);
        assert_eq!(parsed.elementary_stream_map[0].elementary_stream_id, 0xE0);
        assert!(parsed.elementary_stream_map[0]
            .stream_id_extension
            .is_none());
        assert_eq!(
            parsed.elementary_stream_map[0].descriptors,
            &[0x0A, 0x04, b'H', b'E', b'L', b'L']
        );

        // Byte-exact round-trip
        let mut out2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out2).unwrap();
        assert_eq!(&out2[..], &buf[..], "byte-exact round-trip mismatch");

        // Mutation test: change version, output must differ
        let psm_mut = ProgramStreamMap {
            current_next_indicator: true,
            single_extension_stream_flag: false,
            version: 7,
            program_stream_info: &[],
            elementary_stream_map: vec![EsMapEntry {
                stream_type: 0x02,
                elementary_stream_id: 0xE0,
                stream_id_extension: None,
                descriptors: &[0x0A, 0x04, b'H', b'E', b'L', b'L'],
            }],
            crc: 0,
        };
        let mut out3 = vec![0u8; psm_mut.serialized_len()];
        psm_mut.serialize_into(&mut out3).unwrap();
        assert_ne!(&buf[..], &out3[..]);
    }

    #[test]
    fn psm_with_stream_id_extension() {
        let entries = vec![EsMapEntry {
            stream_type: 0x06, // subtitles
            elementary_stream_id: 0xFD,
            stream_id_extension: Some(0x0F),
            descriptors: &[0x59, 0x02, 0x01, 0x02], // subtitling descriptor
        }];

        let psm = ProgramStreamMap {
            current_next_indicator: true,
            single_extension_stream_flag: false,
            version: 1,
            program_stream_info: &[],
            elementary_stream_map: entries,
            crc: 0,
        };

        let mut buf = vec![0u8; psm.serialized_len()];
        psm.serialize_into(&mut buf).unwrap();

        let parsed = ProgramStreamMap::parse(&buf).unwrap();
        assert_eq!(parsed.elementary_stream_map.len(), 1);
        assert_eq!(
            parsed.elementary_stream_map[0].stream_id_extension,
            Some(0x0F)
        );
        assert_eq!(
            parsed.elementary_stream_map[0].descriptors,
            &[0x59, 0x02, 0x01, 0x02]
        );

        // Byte-exact round-trip
        let mut out2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out2).unwrap();
        assert_eq!(&out2[..], &buf[..]);
    }

    #[test]
    fn psm_bad_crc_rejected() {
        let psm = ProgramStreamMap {
            current_next_indicator: true,
            single_extension_stream_flag: false,
            version: 0,
            program_stream_info: &[],
            elementary_stream_map: vec![],
            crc: 0,
        };
        let mut buf = vec![0u8; psm.serialized_len()];
        psm.serialize_into(&mut buf).unwrap();
        // Corrupt CRC (last 4 bytes)
        let crc_off = buf.len() - 4;
        buf[crc_off] ^= 0xFF;
        assert!(matches!(
            ProgramStreamMap::parse(&buf),
            Err(Error::BadCrc { .. })
        ));
    }
}
