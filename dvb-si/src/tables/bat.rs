//! Bouquet Association Table — ETSI EN 300 468 §5.2.2.
//!
//! BAT groups services into operator-defined bouquets ("TNT Sat HD",
//! "Sky DE Sports", "ORF DIGITAL" etc). Carried on PID 0x0011 with
//! table_id 0x4A. Structure mirrors NIT: bouquet-level descriptors +
//! transport_stream loop with per-TS descriptors.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id value for BAT.
pub const TABLE_ID: u8 = 0x4A;
/// Well-known PID on which BAT is carried.
pub const PID: u16 = 0x0011;
/// bouquet_name_descriptor tag (ETSI EN 300 468 §6.2.4).
pub const DESCRIPTOR_TAG_BOUQUET_NAME: u8 = 0x47;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
/// Bytes after the extension header: reserved(4) + bouquet_descriptors_length(12) = 2 bytes.
const POST_EXTENSION_LEN: usize = 2;
const CRC_LEN: usize = 4;
/// Per-transport-stream header: ts_id(2) + original_network_id(2) + reserved(4) + transport_descriptors_length(12) = 6 bytes.
const TS_HEADER_LEN: usize = 6;

/// One transport-stream entry inside the BAT transport_stream_loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BatTransportStream<'a> {
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Raw descriptor bytes for this transport stream.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub descriptors: &'a [u8],
}

/// Bouquet Association Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bat<'a> {
    /// Bouquet identifier (table_id_extension at bytes 3-4).
    pub bouquet_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// Raw bouquet-descriptor bytes (may contain bouquet_name_descriptor 0x47).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub bouquet_descriptors: &'a [u8],
    /// Transport-stream loop entries in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub transport_streams: Vec<BatTransportStream<'a>>,
}

impl<'a> Bat<'a> {
    /// Walk the bouquet_descriptors looking for the first bouquet_name_descriptor
    /// (tag 0x47). Returns the decoded UTF-8 name, or `None` if not present.
    pub fn bouquet_name(&self) -> Option<String> {
        let mut pos = 0usize;
        while pos + 2 <= self.bouquet_descriptors.len() {
            let tag = self.bouquet_descriptors[pos];
            let len = self.bouquet_descriptors[pos + 1] as usize;
            let next = pos + 2 + len;
            if next > self.bouquet_descriptors.len() {
                break;
            }
            if tag == DESCRIPTOR_TAG_BOUQUET_NAME {
                let name_bytes = &self.bouquet_descriptors[pos + 2..next];
                return Some(crate::text::decode(name_bytes).into_owned());
            }
            pos = next;
        }
        None
    }
}

impl<'a> Parse<'a> for Bat<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + 2 + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Bat",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Bat",
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

        // Extension header bytes [3..8]:
        // bytes[3..5] = bouquet_id (table_id_extension)
        // bytes[5]    = reserved(2) | version_number(5) | current_next_indicator(1)
        // bytes[6]    = section_number
        // bytes[7]    = last_section_number
        let bouquet_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        // bytes[8..10] = reserved(4) | bouquet_descriptors_length(12)
        let bouquet_descriptors_length = (((bytes[8] & 0x0F) as usize) << 8) | bytes[9] as usize;

        let bouquet_desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
        let bouquet_desc_end = bouquet_desc_start + bouquet_descriptors_length;

        if bouquet_desc_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: bouquet_descriptors_length,
                available: (total - CRC_LEN).saturating_sub(bouquet_desc_start),
            });
        }

        let bouquet_descriptors = &bytes[bouquet_desc_start..bouquet_desc_end];

        // Transport stream loop: starts right after bouquet_descriptors.
        // First 2 bytes: reserved(4) | transport_stream_loop_length(12).
        let ts_loop_start = bouquet_desc_end;
        let ts_loop_end = total - CRC_LEN;

        if ts_loop_end < ts_loop_start + 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: ts_loop_end - ts_loop_start,
                what: "Bat transport_stream_loop length header",
            });
        }

        let transport_stream_loop_length =
            (((bytes[ts_loop_start] & 0x0F) as usize) << 8) | bytes[ts_loop_start + 1] as usize;

        let loop_end = ts_loop_start + 2 + transport_stream_loop_length;
        if loop_end > ts_loop_end {
            return Err(Error::SectionLengthOverflow {
                declared: transport_stream_loop_length,
                available: ts_loop_end - (ts_loop_start + 2),
            });
        }

        let mut transport_streams = Vec::new();
        let mut pos = ts_loop_start + 2;
        while pos < loop_end {
            if pos + TS_HEADER_LEN > loop_end {
                return Err(Error::BufferTooShort {
                    need: pos + TS_HEADER_LEN,
                    have: loop_end,
                    what: "Bat transport_stream_entry",
                });
            }

            let transport_stream_id = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
            let original_network_id = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]);
            let transport_descriptors_length =
                (((bytes[pos + 4] & 0x0F) as usize) << 8) | bytes[pos + 5] as usize;

            let desc_start = pos + TS_HEADER_LEN;
            let desc_end = desc_start + transport_descriptors_length;

            if desc_end > loop_end {
                return Err(Error::SectionLengthOverflow {
                    declared: transport_descriptors_length,
                    available: loop_end - desc_start,
                });
            }

            transport_streams.push(BatTransportStream {
                transport_stream_id,
                original_network_id,
                descriptors: &bytes[desc_start..desc_end],
            });

            pos = desc_end;
        }

        // CRC is NOT verified here — crate-wide contract: table parsers trust
        // their input and CRC validation is the framing layer's job
        // (`Section::validate_crc`). BAT used to be the lone exception, which
        // made the family contract inconsistent.

        Ok(Bat {
            bouquet_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            bouquet_descriptors,
            transport_streams,
        })
    }
}

impl Serialize for Bat<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let bouquet_desc_len = self.bouquet_descriptors.len();
        let ts_bytes: usize = self
            .transport_streams
            .iter()
            .map(|ts| TS_HEADER_LEN + ts.descriptors.len())
            .sum();
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + POST_EXTENSION_LEN
            + bouquet_desc_len
            + 2 // transport_stream_loop_length header
            + ts_bytes
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

        // Extension header.
        buf[3..5].copy_from_slice(&self.bouquet_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        // Bouquet descriptors length field.
        let bdl = self.bouquet_descriptors.len() as u16;
        buf[8] = 0xF0 | ((bdl >> 8) as u8 & 0x0F);
        buf[9] = (bdl & 0xFF) as u8;

        let bouquet_desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
        buf[bouquet_desc_start..bouquet_desc_start + self.bouquet_descriptors.len()]
            .copy_from_slice(self.bouquet_descriptors);

        let ts_loop_start = bouquet_desc_start + self.bouquet_descriptors.len();
        let ts_loop_length: u16 = (len - ts_loop_start - 2 - CRC_LEN) as u16;
        buf[ts_loop_start] = 0xF0 | ((ts_loop_length >> 8) as u8 & 0x0F);
        buf[ts_loop_start + 1] = (ts_loop_length & 0xFF) as u8;

        let mut pos = ts_loop_start + 2;
        for ts in &self.transport_streams {
            buf[pos..pos + 2].copy_from_slice(&ts.transport_stream_id.to_be_bytes());
            buf[pos + 2..pos + 4].copy_from_slice(&ts.original_network_id.to_be_bytes());
            let tdl = ts.descriptors.len() as u16;
            buf[pos + 4] = 0xF0 | ((tdl >> 8) as u8 & 0x0F);
            buf[pos + 5] = (tdl & 0xFF) as u8;
            let desc_start = pos + TS_HEADER_LEN;
            buf[desc_start..desc_start + ts.descriptors.len()].copy_from_slice(ts.descriptors);
            pos = desc_start + ts.descriptors.len();
        }

        // CRC: compute over everything up to (but not including) the CRC slot.
        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for Bat<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Bat<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "BOUQUET_ASSOCIATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestTs = (u16, u16, Vec<u8>);

    /// Build a complete BAT section (with valid CRC).
    fn build_bat(
        bouquet_id: u16,
        version: u8,
        section_number: u8,
        last_section_number: u8,
        bouquet_desc: &[u8],
        transport_streams: &[TestTs],
    ) -> Vec<u8> {
        let ts_streams: Vec<BatTransportStream> = transport_streams
            .iter()
            .map(|(tsid, onid, d)| BatTransportStream {
                transport_stream_id: *tsid,
                original_network_id: *onid,
                descriptors: d,
            })
            .collect();
        let bat = Bat {
            bouquet_id,
            version_number: version,
            current_next_indicator: true,
            section_number,
            last_section_number,
            bouquet_descriptors: bouquet_desc,
            transport_streams: ts_streams,
        };
        let mut buf = vec![0u8; bat.serialized_len()];
        bat.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn parse_extracts_bouquet_id() {
        let bytes = build_bat(0x1234, 3, 0, 0, &[], &[]);
        let bat = Bat::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_id, 0x1234);
    }

    #[test]
    fn parse_extracts_version_and_cni() {
        let bytes = build_bat(0x0001, 7, 0, 0, &[], &[]);
        let bat = Bat::parse(&bytes).unwrap();
        assert_eq!(bat.version_number, 7);
        assert!(bat.current_next_indicator);
    }

    #[test]
    fn parse_extracts_section_numbers() {
        let bytes = build_bat(0x0001, 0, 2, 4, &[], &[]);
        let bat = Bat::parse(&bytes).unwrap();
        assert_eq!(bat.section_number, 2);
        assert_eq!(bat.last_section_number, 4);
    }

    #[test]
    fn bouquet_name_descriptor_extracted() {
        let name_desc: Vec<u8> = vec![
            DESCRIPTOR_TAG_BOUQUET_NAME,
            0x05,
            b'H',
            b'E',
            b'L',
            b'L',
            b'O',
        ];
        let bytes = build_bat(0x0001, 0, 0, 0, &name_desc, &[]);
        let bat = Bat::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_name(), Some("HELLO".to_string()));
    }

    #[test]
    fn bouquet_name_returns_none_when_no_bouquet_name_descriptor() {
        // Non-bouquet-name descriptor (tag 0x40 = network_name, also human-readable).
        let other_desc: Vec<u8> = vec![0x40, 0x03, b'A', b'B', b'C'];
        let bytes = build_bat(0x0001, 0, 0, 0, &other_desc, &[]);
        let bat = Bat::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_name(), None);
    }

    #[test]
    fn private_descriptors_preserved_in_bouquet_descriptors() {
        // Private descriptor tag (>= 0x80). Per anti-instructions, surface as raw bytes.
        let private_desc: Vec<u8> = vec![0x80, 0x04, 0xDE, 0xAD, 0xBE, 0xEF];
        let bytes = build_bat(0x0001, 0, 0, 0, &private_desc, &[]);
        let bat = Bat::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_descriptors, &private_desc[..]);
    }

    #[test]
    fn parse_transport_stream_entries() {
        let bytes = build_bat(
            0x0001,
            0,
            0,
            0,
            &[],
            &[
                (
                    0x1234,
                    0x0020,
                    vec![0x43, 0x07, 0x0B, 0xB8, 0x00, 0x02, 0x00, 0x05],
                ),
                (0x5678, 0x0020, vec![]),
            ],
        );
        let bat = Bat::parse(&bytes).unwrap();
        assert_eq!(bat.transport_streams.len(), 2);
        assert_eq!(bat.transport_streams[0].transport_stream_id, 0x1234);
        assert_eq!(bat.transport_streams[0].original_network_id, 0x0020);
        assert_eq!(
            bat.transport_streams[0].descriptors,
            &[0x43, 0x07, 0x0B, 0xB8, 0x00, 0x02, 0x00, 0x05][..]
        );
        assert_eq!(bat.transport_streams[1].transport_stream_id, 0x5678);
        assert_eq!(bat.transport_streams[1].descriptors.len(), 0);
    }

    #[test]
    fn bat_with_no_transport_streams_parses_ok() {
        let bytes = build_bat(0x0001, 0, 0, 0, &[], &[]);
        let bat = Bat::parse(&bytes).unwrap();
        assert!(bat.transport_streams.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let name_desc: Vec<u8> = vec![DESCRIPTOR_TAG_BOUQUET_NAME, 0x04, b'T', b'E', b'S', b'T'];
        let ts_desc: [u8; 3] = [0x43, 0x01, 0x01];
        let bat = Bat {
            bouquet_id: 0x4242,
            version_number: 5,
            current_next_indicator: true,
            section_number: 1,
            last_section_number: 2,
            bouquet_descriptors: &name_desc,
            transport_streams: vec![
                BatTransportStream {
                    transport_stream_id: 0x1234,
                    original_network_id: 0x0020,
                    descriptors: &ts_desc,
                },
                BatTransportStream {
                    transport_stream_id: 0x5678,
                    original_network_id: 0x0020,
                    descriptors: &[],
                },
            ],
        };
        let mut buf = vec![0u8; bat.serialized_len()];
        bat.serialize_into(&mut buf).unwrap();
        let parsed = Bat::parse(&buf).unwrap();
        assert_eq!(bat, parsed);
    }

    /// Crate-wide contract: table parsers do NOT verify CRC — that is the
    /// framing layer's job (`Section::validate_crc`). A corrupted-CRC BAT
    /// still parses; callers wanting integrity check the Section first.
    #[test]
    fn bat_parse_does_not_verify_crc() {
        let mut bytes = build_bat(0x0001, 0, 0, 0, &[], &[]);
        bytes[3] ^= 0xFF; // corrupt bouquet_id high byte → CRC now wrong
        let bat = Bat::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_id, 0x0001 ^ 0xFF00);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Bat::parse(&[0x4A, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_bat(0x0001, 0, 0, 0, &[], &[]);
        bytes[0] = 0x00;
        let err = Bat::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x00, .. }
        ));
    }

    #[test]
    fn serialize_too_small_buffer_returns_error() {
        let bat = Bat {
            bouquet_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            bouquet_descriptors: &[],
            transport_streams: vec![],
        };
        let mut buf = vec![0u8; 2];
        let err = bat.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
