//! Bouquet Association Table — ETSI EN 300 468 §5.2.2.
//!
//! BAT groups services into operator-defined bouquets ("TNT SatSection HD",
//! "Sky DE Sports", "ORF DIGITAL" etc). Carried on PID 0x0011 with
//! table_id 0x4A. Structure mirrors NIT: bouquet-level descriptors +
//! transport_stream loop with per-TS descriptors.

use super::ts_loop_section::{
    self, LOOP_LENGTH_FIELD_LEN, Labels, MIN_SECTION_LEN, TransportStreamEntry,
};
use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;
use broadcast_common::len::need_in;
use broadcast_common::{Parse, Serialize};

/// table_id value for BAT.
pub const TABLE_ID: u8 = 0x4A;
/// Well-known PID on which BAT is carried.
pub const PID: mpeg_ts::pid::Pid = mpeg_ts::pid::Pid::new(0x0011);
/// bouquet_name_descriptor tag (ETSI EN 300 468 §6.2.4).
pub const DESCRIPTOR_TAG_BOUQUET_NAME: u8 = 0x47;

/// Bouquet Association Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct BatSection<'a> {
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
    /// Bouquet descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub bouquet_descriptors: DescriptorLoop<'a>,
    /// Transport-stream loop entries in wire order.
    pub transport_streams: Vec<TransportStreamEntry<'a>>,
    /// Header bit fields (`reserved_future_use`, `reserved(2)` x2; default
    /// [`SectionBits::dvb`](super::SectionBits::dvb)).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bits: super::SectionBits,
    /// `reserved_future_use(4)` before `bouquet_descriptors_length` (`1111`).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_bouquet_descriptors_length: u8,
    /// `reserved_future_use(4)` before `transport_stream_loop_length` (`1111`).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_transport_stream_loop_length: u8,
}

impl<'a> BatSection<'a> {
    /// Construct a BAT section with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        bouquet_id: u16,
        version_number: u8,
        current_next_indicator: bool,
        section_number: u8,
        last_section_number: u8,
        bouquet_descriptors: DescriptorLoop<'a>,
        transport_streams: Vec<TransportStreamEntry<'a>>,
    ) -> Self {
        Self {
            bouquet_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            bouquet_descriptors,
            transport_streams,
            bits: super::SectionBits::dvb(),
            reserved_bouquet_descriptors_length: super::RESERVED_4,
            reserved_transport_stream_loop_length: super::RESERVED_4,
        }
    }
}

impl BatSection<'_> {
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

const LABELS: Labels = Labels {
    section: "BatSection",
    loop_header: "BatSection transport_stream_loop length header",
    trailing: "BatSection trailing bytes after transport stream loop",
    entry: "BatSection transport_stream_entry",
    extension: "bat",
};

impl<'a> Parse<'a> for BatSection<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_SECTION_LEN + LOOP_LENGTH_FIELD_LEN;
        need_in(bytes, min_len, LABELS.section)?;
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: LABELS.section,
                expected: &[TABLE_ID],
            });
        }
        let p = ts_loop_section::parse(bytes, &LABELS)?;
        Ok(BatSection {
            bouquet_id: p.id,
            version_number: p.version_number,
            current_next_indicator: p.current_next_indicator,
            section_number: p.section_number,
            last_section_number: p.last_section_number,
            bouquet_descriptors: p.descriptors,
            transport_streams: p.transport_streams,
            bits: p.bits,
            reserved_bouquet_descriptors_length: p.reserved_descriptors_length,
            reserved_transport_stream_loop_length: p.reserved_transport_stream_loop_length,
        })
    }
}

impl Serialize for BatSection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        ts_loop_section::serialized_len(&self.bouquet_descriptors, &self.transport_streams)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        ts_loop_section::serialize_into(
            TABLE_ID,
            &ts_loop_section::View {
                id: self.bouquet_id,
                version_number: self.version_number,
                current_next_indicator: self.current_next_indicator,
                section_number: self.section_number,
                last_section_number: self.last_section_number,
                descriptors: &self.bouquet_descriptors,
                transport_streams: &self.transport_streams,
                bits: &self.bits,
                reserved_descriptors_length: self.reserved_bouquet_descriptors_length,
                reserved_transport_stream_loop_length: self.reserved_transport_stream_loop_length,
                descriptors_length_what: "bouquet_descriptors_length",
            },
            buf,
        )
    }
}
impl<'a> crate::traits::TableDef<'a> for BatSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "BOUQUET_ASSOCIATION";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{string::ToString, vec};

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
        let ts_streams: Vec<TransportStreamEntry> = transport_streams
            .iter()
            .map(|(tsid, onid, d)| TransportStreamEntry::new(*tsid, *onid, DescriptorLoop::new(d)))
            .collect();
        let bat = BatSection::new(
            bouquet_id,
            version,
            true,
            section_number,
            last_section_number,
            DescriptorLoop::new(bouquet_desc),
            ts_streams,
        );
        let mut buf = vec![0u8; bat.serialized_len()];
        bat.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn parse_extracts_bouquet_id() {
        let bytes = build_bat(0x1234, 3, 0, 0, &[], &[]);
        let bat = BatSection::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_id, 0x1234);
    }

    #[test]
    fn parse_extracts_version_and_cni() {
        let bytes = build_bat(0x0001, 7, 0, 0, &[], &[]);
        let bat = BatSection::parse(&bytes).unwrap();
        assert_eq!(bat.version_number, 7);
        assert!(bat.current_next_indicator);
    }

    #[test]
    fn parse_extracts_section_numbers() {
        let bytes = build_bat(0x0001, 0, 2, 4, &[], &[]);
        let bat = BatSection::parse(&bytes).unwrap();
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
        let bat = BatSection::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_name(), Some("HELLO".to_string()));
    }

    #[test]
    fn bouquet_name_returns_none_when_no_bouquet_name_descriptor() {
        // Non-bouquet-name descriptor (tag 0x40 = network_name, also human-readable).
        let other_desc: Vec<u8> = vec![0x40, 0x03, b'A', b'B', b'C'];
        let bytes = build_bat(0x0001, 0, 0, 0, &other_desc, &[]);
        let bat = BatSection::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_name(), None);
    }

    #[test]
    fn private_descriptors_preserved_in_bouquet_descriptors() {
        // Private descriptor tag (>= 0x80). Per anti-instructions, surface as raw bytes.
        let private_desc: Vec<u8> = vec![0x80, 0x04, 0xDE, 0xAD, 0xBE, 0xEF];
        let bytes = build_bat(0x0001, 0, 0, 0, &private_desc, &[]);
        let bat = BatSection::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_descriptors.raw(), &private_desc[..]);
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
        let bat = BatSection::parse(&bytes).unwrap();
        assert_eq!(bat.transport_streams.len(), 2);
        assert_eq!(bat.transport_streams[0].transport_stream_id, 0x1234);
        assert_eq!(bat.transport_streams[0].original_network_id, 0x0020);
        assert_eq!(
            bat.transport_streams[0].descriptors.raw(),
            &[0x43, 0x07, 0x0B, 0xB8, 0x00, 0x02, 0x00, 0x05][..]
        );
        assert_eq!(bat.transport_streams[1].transport_stream_id, 0x5678);
        assert_eq!(bat.transport_streams[1].descriptors.len(), 0);
    }

    #[test]
    fn bat_with_no_transport_streams_parses_ok() {
        let bytes = build_bat(0x0001, 0, 0, 0, &[], &[]);
        let bat = BatSection::parse(&bytes).unwrap();
        assert!(bat.transport_streams.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let name_desc: Vec<u8> = vec![DESCRIPTOR_TAG_BOUQUET_NAME, 0x04, b'T', b'E', b'S', b'T'];
        let ts_desc: [u8; 3] = [0x43, 0x01, 0x01];
        let bat = BatSection::new(
            0x4242,
            5,
            true,
            1,
            2,
            DescriptorLoop::new(&name_desc),
            vec![
                TransportStreamEntry::new(0x1234, 0x0020, DescriptorLoop::new(&ts_desc)),
                TransportStreamEntry::new(0x5678, 0x0020, DescriptorLoop::new(&[])),
            ],
        );
        let mut buf = vec![0u8; bat.serialized_len()];
        bat.serialize_into(&mut buf).unwrap();
        let parsed = BatSection::parse(&buf).unwrap();
        assert_eq!(bat, parsed);
    }

    /// Crate-wide contract: table parsers do NOT verify CRC — that is the
    /// framing layer's job (`Section::validate_crc`). A corrupted-CRC BAT
    /// still parses; callers wanting integrity check the Section first.
    #[test]
    fn bat_parse_does_not_verify_crc() {
        let mut bytes = build_bat(0x0001, 0, 0, 0, &[], &[]);
        bytes[3] ^= 0xFF; // corrupt bouquet_id high byte → CRC now wrong
        let bat = BatSection::parse(&bytes).unwrap();
        assert_eq!(bat.bouquet_id, 0x0001 ^ 0xFF00);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = BatSection::parse(&[0x4A, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort(..)));
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_bat(0x0001, 0, 0, 0, &[], &[]);
        bytes[0] = 0x00;
        let err = BatSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x00, .. }
        ));
    }

    #[test]
    fn serialize_too_small_buffer_returns_error() {
        let bat = BatSection::new(0x0001, 0, true, 0, 0, DescriptorLoop::new(&[]), vec![]);
        let mut buf = vec![0u8; 2];
        let err = bat.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall(..)));
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
            BatSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    /// section_length is 12 bits (max 4095 = EXTENSION_HEADER_LEN(5) +
    /// POST_EXTENSION_LEN(2) + bouquet_desc + ts_loop_length field(2) +
    /// CRC(4), so 4082 is the largest bouquet_descriptors loop that fits
    /// with an empty transport_stream_loop).
    #[test]
    fn serialize_rejects_bouquet_descriptors_exceeding_section_length() {
        let desc = vec![0xAAu8; 4083];
        let bat = BatSection::new(1, 0, true, 0, 0, DescriptorLoop::new(&desc), vec![]);
        let mut buf = vec![0u8; bat.serialized_len()];
        assert!(matches!(
            bat.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[test]
    fn serialize_accepts_bouquet_descriptors_at_section_length_boundary() {
        let desc = vec![0xAAu8; 4082];
        let bat = BatSection::new(1, 0, true, 0, 0, DescriptorLoop::new(&desc), vec![]);
        let mut buf = vec![0u8; bat.serialized_len()];
        bat.serialize_into(&mut buf).expect("boundary fits");
        let reparsed = BatSection::parse(&buf).expect("reparse");
        assert_eq!(
            bat.bouquet_descriptors.raw(),
            reparsed.bouquet_descriptors.raw()
        );
    }
}
