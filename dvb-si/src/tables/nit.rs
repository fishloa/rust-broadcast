//! Network Information Table — ETSI EN 300 468 §5.2.1.
//!
//! NIT describes the transport streams belonging to a DVB network, with
//! network-wide descriptors and per-transport-stream descriptors. The
//! table is split into two variants by table_id: `0x40` for the actual
//! TS the receiver is tuned to, `0x41` for other TSes in the same network.

use super::ts_loop_section::{self, Labels, MIN_SECTION_LEN, TransportStreamEntry};
use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::len::need_in;
use broadcast_common::{Parse, Serialize};

/// table_id value for NIT actual (current TS).
pub const TABLE_ID_ACTUAL: u8 = 0x40;
/// table_id value for NIT other (other TSes in the network).
pub const TABLE_ID_OTHER: u8 = 0x41;
/// Well-known PID on which NIT is carried.
pub const PID: mpeg_ts::pid::Pid = mpeg_ts::pid::Pid::from_const(0x0010);

/// NIT kind — distinguishes `0x40` (actual) from `0x41` (other).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum NitKind {
    /// NIT for the transport stream the receiver is tuned to.
    Actual,
    /// NIT describing other transport streams in the same network.
    Other,
}

/// Network Information Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct NitSection<'a> {
    /// Variant discriminator (table_id 0x40 vs 0x41).
    pub kind: NitKind,
    /// Network identifier.
    pub network_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// Raw network-wide descriptor bytes.
    /// Network descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub network_descriptors: DescriptorLoop<'a>,
    /// Transport-stream loop entries in wire order.
    pub transport_streams: Vec<TransportStreamEntry<'a>>,
    /// Header bit fields (`reserved_future_use`, `reserved(2)` x2; default
    /// [`SectionBits::dvb`](super::SectionBits::dvb)).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bits: super::SectionBits,
    /// `reserved_future_use(4)` before `network_descriptors_length` (`1111`).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_network_descriptors_length: u8,
    /// `reserved_future_use(4)` before `transport_stream_loop_length` (`1111`).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_transport_stream_loop_length: u8,
}

impl<'a> NitSection<'a> {
    /// Construct a NIT section with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        kind: NitKind,
        network_id: u16,
        section: crate::tables::SectionVersion,
        network_descriptors: DescriptorLoop<'a>,
        transport_streams: Vec<TransportStreamEntry<'a>>,
    ) -> Self {
        Self {
            kind,
            network_id,
            version_number: section.version_number,
            current_next_indicator: section.current_next_indicator,
            section_number: section.section_number,
            last_section_number: section.last_section_number,
            network_descriptors,
            transport_streams,
            bits: super::SectionBits::dvb(),
            reserved_network_descriptors_length: super::RESERVED_4,
            reserved_transport_stream_loop_length: super::RESERVED_4,
        }
    }
}

const LABELS: Labels = Labels {
    section: "NitSection",
    loop_header: "NitSection transport_stream_loop",
    trailing: "NitSection trailing bytes after transport stream loop",
    entry: "NitSection transport_stream_entry",
    extension: "nit",
};

impl<'a> Parse<'a> for NitSection<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, MIN_SECTION_LEN, LABELS.section)?;
        let kind = match bytes[0] {
            TABLE_ID_ACTUAL => NitKind::Actual,
            TABLE_ID_OTHER => NitKind::Other,
            other => {
                return Err(Error::UnexpectedTableId {
                    table_id: other,
                    what: LABELS.section,
                    expected: &[TABLE_ID_ACTUAL, TABLE_ID_OTHER],
                });
            }
        };
        let p = ts_loop_section::parse(bytes, &LABELS)?;
        Ok(NitSection {
            kind,
            network_id: p.id,
            version_number: p.version_number,
            current_next_indicator: p.current_next_indicator,
            section_number: p.section_number,
            last_section_number: p.last_section_number,
            network_descriptors: p.descriptors,
            transport_streams: p.transport_streams,
            bits: p.bits,
            reserved_network_descriptors_length: p.reserved_descriptors_length,
            reserved_transport_stream_loop_length: p.reserved_transport_stream_loop_length,
        })
    }
}

impl Serialize for NitSection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        ts_loop_section::serialized_len(&self.network_descriptors, &self.transport_streams)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let table_id = match self.kind {
            NitKind::Actual => TABLE_ID_ACTUAL,
            NitKind::Other => TABLE_ID_OTHER,
        };
        ts_loop_section::serialize_into(
            table_id,
            &ts_loop_section::View {
                id: self.network_id,
                version_number: self.version_number,
                current_next_indicator: self.current_next_indicator,
                section_number: self.section_number,
                last_section_number: self.last_section_number,
                descriptors: &self.network_descriptors,
                transport_streams: &self.transport_streams,
                bits: &self.bits,
                reserved_descriptors_length: self.reserved_network_descriptors_length,
                reserved_transport_stream_loop_length: self.reserved_transport_stream_loop_length,
                descriptors_length_what: "network_descriptors_length",
            },
            buf,
        )
    }
}
impl<'a> crate::traits::TableDef<'a> for NitSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID_ACTUAL, TABLE_ID_OTHER)];
    const NAME: &'static str = "NETWORK_INFORMATION";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::CRC_LEN;
    use crate::tables::ts_loop_section::{EXTENSION_HEADER_LEN, POST_EXTENSION_LEN, TS_HEADER_LEN};
    use alloc::vec;

    type TestTs = (u16, u16, Vec<u8>);

    fn build_nit(
        kind: NitKind,
        network_id: u16,
        network_desc: &[u8],
        transport_streams: &[TestTs],
    ) -> Vec<u8> {
        let ts_bytes: usize = transport_streams
            .iter()
            .map(|(_, _, d)| TS_HEADER_LEN + d.len())
            .sum();
        let loop_length = ts_bytes as u16;
        let section_length: u16 = (EXTENSION_HEADER_LEN
            + POST_EXTENSION_LEN
            + network_desc.len()
            + 2
            + ts_bytes
            + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(match kind {
            NitKind::Actual => TABLE_ID_ACTUAL,
            NitKind::Other => TABLE_ID_OTHER,
        });
        crate::tables::push_section_header(
            &mut v,
            crate::tables::SECTION_B1_FLAGS_DVB,
            section_length as usize,
        );
        // Extension header (spec layout): bytes[3..5] = network_id
        v.extend_from_slice(&network_id.to_be_bytes());
        v.push(crate::tables::version_byte(0, true)); // version=0, current_next=1
        v.push(0); // section_number
        v.push(0); // last_section_number
        // bytes[8..10] = reserved(4) | network_descriptors_length(12)
        crate::tables::push_desc_loop_len(&mut v, network_desc.len());
        v.extend_from_slice(network_desc);
        // reserved(4) | transport_stream_loop_length(12)
        crate::tables::push_desc_loop_len(&mut v, loop_length as usize);
        for (tsid, onid, desc) in transport_streams {
            v.extend_from_slice(&tsid.to_be_bytes());
            v.extend_from_slice(&onid.to_be_bytes());
            // reserved(4) | transport_descriptors_length(12)
            crate::tables::push_desc_loop_len(&mut v, desc.len());
            v.extend_from_slice(desc);
        }
        v.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
        v
    }

    #[test]
    fn parse_actual_and_other_distinguished_by_table_id() {
        let a = build_nit(NitKind::Actual, 0x0001, &[], &[]);
        let o = build_nit(NitKind::Other, 0x0001, &[], &[]);
        assert!(matches!(
            NitSection::parse(&a).unwrap().kind,
            NitKind::Actual
        ));
        assert!(matches!(
            NitSection::parse(&o).unwrap().kind,
            NitKind::Other
        ));
    }

    #[test]
    fn parse_network_id_extracted() {
        let bytes = build_nit(NitKind::Actual, 0x0020, &[], &[]);
        let nit = NitSection::parse(&bytes).unwrap();
        assert_eq!(nit.network_id, 0x0020);
    }

    #[test]
    fn parse_network_wide_descriptors() {
        let net_desc: [u8; 4] = [0x40, 0x02, 0x4E, 0x65]; // network_name descriptor
        let bytes = build_nit(NitKind::Actual, 0x0001, &net_desc, &[]);
        let nit = NitSection::parse(&bytes).unwrap();
        assert_eq!(nit.network_descriptors.raw(), &net_desc[..]);
    }

    #[test]
    fn parse_transport_stream_entries() {
        let bytes = build_nit(
            NitKind::Actual,
            0x0001,
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
        let nit = NitSection::parse(&bytes).unwrap();
        assert_eq!(nit.transport_streams.len(), 2);
        assert_eq!(nit.transport_streams[0].transport_stream_id, 0x1234);
        assert_eq!(nit.transport_streams[0].original_network_id, 0x0020);
        assert_eq!(
            nit.transport_streams[0].descriptors.raw(),
            &[0x43, 0x07, 0x0B, 0xB8, 0x00, 0x02, 0x00, 0x05][..]
        );
        assert_eq!(nit.transport_streams[1].transport_stream_id, 0x5678);
        assert_eq!(nit.transport_streams[1].descriptors.len(), 0);
    }

    #[test]
    fn parse_version_and_current_next() {
        let bytes = build_nit(NitKind::Actual, 0x0001, &[], &[]);
        let nit = NitSection::parse(&bytes).unwrap();
        assert_eq!(nit.version_number, 0);
        assert!(nit.current_next_indicator);
    }

    #[test]
    fn serialize_round_trip() {
        let net_desc: [u8; 4] = [0x40, 0x02, 0x4E, 0x65];
        let ts_desc: [u8; 3] = [0x43, 0x01, 0x01];
        let nit = NitSection::new(
            NitKind::Actual,
            0x0020,
            crate::tables::SectionVersion::new(3, true, 0, 0),
            DescriptorLoop::new(&net_desc),
            vec![
                TransportStreamEntry::new(0x1234, 0x0020, DescriptorLoop::new(&ts_desc)),
                TransportStreamEntry::new(0x5678, 0x0020, DescriptorLoop::new(&[])),
            ],
        );
        let mut buf = vec![0u8; nit.serialized_len()];
        nit.serialize_into(&mut buf).unwrap();
        let re = NitSection::parse(&buf).unwrap();
        assert_eq!(nit, re);
    }

    #[test]
    fn zero_transport_streams_is_valid() {
        let bytes = build_nit(NitKind::Actual, 0x0001, &[], &[]);
        let nit = NitSection::parse(&bytes).unwrap();
        assert_eq!(nit.transport_streams.len(), 0);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = NitSection::parse(&[0x40, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort(..)));
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_nit(NitKind::Actual, 0x0001, &[], &[]);
        bytes[0] = 0x00;
        let err = NitSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x00, .. }
        ));
    }

    #[test]
    fn serialize_too_small_buffer_returns_error() {
        let nit = NitSection::new(
            NitKind::Actual,
            0x0001,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            DescriptorLoop::new(&[]),
            vec![],
        );
        let mut buf = vec![0u8; 2];
        let err = nit.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall(..)));
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID_ACTUAL;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            NitSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    /// section_length is 12 bits (max 4095 = EXTENSION_HEADER_LEN(5) +
    /// POST_EXTENSION_LEN(2) + net_desc + ts_loop_length field(2) + CRC(4),
    /// so 4082 is the largest network_descriptors loop that fits with an
    /// empty transport_stream_loop).
    #[test]
    fn serialize_rejects_network_descriptors_exceeding_section_length() {
        let net_desc = vec![0xAAu8; 4083];
        let nit = NitSection::new(
            NitKind::Actual,
            1,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            DescriptorLoop::new(&net_desc),
            vec![],
        );
        let mut buf = vec![0u8; nit.serialized_len()];
        assert!(matches!(
            nit.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[test]
    fn serialize_accepts_network_descriptors_at_section_length_boundary() {
        let net_desc = vec![0xAAu8; 4082];
        let nit = NitSection::new(
            NitKind::Actual,
            1,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            DescriptorLoop::new(&net_desc),
            vec![],
        );
        let mut buf = vec![0u8; nit.serialized_len()];
        nit.serialize_into(&mut buf).expect("boundary fits");
        let reparsed = NitSection::parse(&buf).expect("reparse");
        assert_eq!(
            nit.network_descriptors.raw(),
            reparsed.network_descriptors.raw()
        );
    }
}
