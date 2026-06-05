//! Network Information Table — ETSI EN 300 468 §5.2.1.
//!
//! NIT describes the transport streams belonging to a DVB network, with
//! network-wide descriptors and per-transport-stream descriptors. The
//! table is split into two variants by table_id: `0x40` for the actual
//! TS the receiver is tuned to, `0x41` for other TSes in the same network.

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id value for NIT actual (current TS).
pub const TABLE_ID_ACTUAL: u8 = 0x40;
/// table_id value for NIT other (other TSes in the network).
pub const TABLE_ID_OTHER: u8 = 0x41;
/// Well-known PID on which NIT is carried.
pub const PID: u16 = 0x0010;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
/// Bytes after the extension header: reserved_future_use (4 bits) + network_descriptors_length (12 bits) = 2 bytes.
/// (Previously incorrectly defined as 4 — the parser assumed `network_id`
/// was duplicated *after* the extension header; spec says it lives IN
/// the extension header at bytes 3-4 as `table_id_extension`.)
const POST_EXTENSION_LEN: usize = 2;
const CRC_LEN: usize = 4;
/// Per-transport-stream header: ts_id (2) + original_network_id (2) + reserved_future_use (4 bits) + transport_descriptors_length (12 bits).
const TS_HEADER_LEN: usize = 6;

/// NIT kind — distinguishes `0x40` (actual) from `0x41` (other).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum NitKind {
    /// NIT for the transport stream the receiver is tuned to.
    Actual,
    /// NIT describing other transport streams in the same network.
    Other,
}

/// One transport-stream entry inside the NIT transport_stream_loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NitTransportStream<'a> {
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Raw descriptor bytes for this transport stream.
    /// Per-TS descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub descriptors: DescriptorLoop<'a>,
}

/// Network Information Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Nit<'a> {
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
    pub transport_streams: Vec<NitTransportStream<'a>>,
}

impl<'a> Parse<'a> for Nit<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Nit",
            });
        }
        let kind = match bytes[0] {
            TABLE_ID_ACTUAL => NitKind::Actual,
            TABLE_ID_OTHER => NitKind::Other,
            other => {
                return Err(Error::UnexpectedTableId {
                    table_id: other,
                    what: "Nit",
                    expected: &[TABLE_ID_ACTUAL, TABLE_ID_OTHER],
                });
            }
        };

        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = MIN_HEADER_LEN + section_length as usize;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: bytes.len() - MIN_HEADER_LEN,
            });
        }

        // Extension header bytes [3..8] per ETSI EN 300 468 §5.2.1:
        //   bytes[3..5] = table_id_extension = network_id (16 bits)
        //   bytes[5]    = reserved(2) | version_number(5) | current_next_indicator(1)
        //   bytes[6]    = section_number
        //   bytes[7]    = last_section_number
        let network_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        // bytes[8..10] = reserved(4) | network_descriptors_length(12)
        let network_descriptors_length = (((bytes[8] & 0x0F) as usize) << 8) | bytes[9] as usize;

        let network_desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
        let network_desc_end = network_desc_start + network_descriptors_length;

        // Verify network descriptors don't overflow
        if network_desc_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: network_descriptors_length,
                available: (total - CRC_LEN) - network_desc_start,
            });
        }

        let network_descriptors = DescriptorLoop::new(&bytes[network_desc_start..network_desc_end]);

        // Parse transport stream loop starting after network descriptors
        let ts_loop_start = network_desc_end;
        let ts_loop_end = total - CRC_LEN;

        // First 2 bytes of the loop are: reserved_future_use (4 bits) + transport_stream_loop_length (12 bits)
        if ts_loop_end - ts_loop_start < 2 {
            return Err(Error::BufferTooShort {
                need: ts_loop_end - ts_loop_start + 2,
                have: ts_loop_end - ts_loop_start,
                what: "Nit transport_stream_loop",
            });
        }

        let transport_stream_loop_length =
            (((bytes[ts_loop_start] & 0x0F) as usize) << 8) | bytes[ts_loop_start + 1] as usize;

        let mut pos = ts_loop_start + 2;
        let loop_end = ts_loop_start + 2 + transport_stream_loop_length;

        if loop_end > ts_loop_end {
            return Err(Error::SectionLengthOverflow {
                declared: transport_stream_loop_length,
                available: ts_loop_end - (ts_loop_start + 2),
            });
        }

        let mut transport_streams = Vec::new();

        while pos < loop_end {
            if pos + TS_HEADER_LEN > loop_end {
                return Err(Error::BufferTooShort {
                    need: pos + TS_HEADER_LEN,
                    have: loop_end,
                    what: "Nit transport_stream_entry",
                });
            }

            let transport_stream_id = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
            let original_network_id = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]);

            // transport_descriptors_length is 12 bits: high 4 bits reserved, low 12 bits length
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

            transport_streams.push(NitTransportStream {
                transport_stream_id,
                original_network_id,
                descriptors: DescriptorLoop::new(&bytes[desc_start..desc_end]),
            });

            pos = desc_end;
        }

        Ok(Nit {
            kind,
            network_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            network_descriptors,
            transport_streams,
        })
    }
}

impl Serialize for Nit<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let net_desc_len = self.network_descriptors.len();
        let ts_bytes: usize = self
            .transport_streams
            .iter()
            .map(|ts| TS_HEADER_LEN + ts.descriptors.len())
            .sum();
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + POST_EXTENSION_LEN
            + net_desc_len
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
        buf[0] = match self.kind {
            NitKind::Actual => TABLE_ID_ACTUAL,
            NitKind::Other => TABLE_ID_OTHER,
        };
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        // Extension header per ETSI EN 300 468 §5.2.1:
        //   bytes[3..5] = network_id (table_id_extension)
        //   bytes[5]    = reserved | version | current_next
        //   bytes[6..8] = section_number + last_section_number
        buf[3..5].copy_from_slice(&self.network_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        // bytes[8..10] = reserved(4) | network_descriptors_length(12)
        let net_dll = self.network_descriptors.len() as u16;
        buf[8] = 0xF0 | ((net_dll >> 8) as u8 & 0x0F);
        buf[9] = (net_dll & 0xFF) as u8;

        let net_desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
        buf[net_desc_start..net_desc_start + self.network_descriptors.len()]
            .copy_from_slice(self.network_descriptors.raw());

        let ts_loop_start = net_desc_start + self.network_descriptors.len();
        let ts_loop_length: u16 = (len - ts_loop_start - 2 - CRC_LEN) as u16;
        buf[ts_loop_start] = 0xF0 | ((ts_loop_length >> 8) as u8 & 0x0F);
        buf[ts_loop_start + 1] = (ts_loop_length & 0xFF) as u8;

        let mut pos = ts_loop_start + 2;
        for ts in &self.transport_streams {
            buf[pos..pos + 2].copy_from_slice(&ts.transport_stream_id.to_be_bytes());
            buf[pos + 2..pos + 4].copy_from_slice(&ts.original_network_id.to_be_bytes());
            let ts_dll = ts.descriptors.len() as u16;
            buf[pos + 4] = 0xF0 | ((ts_dll >> 8) as u8 & 0x0F);
            buf[pos + 5] = (ts_dll & 0xFF) as u8;
            let desc_start = pos + TS_HEADER_LEN;
            buf[desc_start..desc_start + ts.descriptors.len()]
                .copy_from_slice(ts.descriptors.raw());
            pos = desc_start + ts.descriptors.len();
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for Nit<'a> {
    const TABLE_ID: u8 = TABLE_ID_ACTUAL;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Nit<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID_ACTUAL, TABLE_ID_OTHER)];
    const NAME: &'static str = "NETWORK_INFORMATION";
}

#[cfg(test)]
mod tests {
    use super::*;

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
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        // Extension header (spec layout): bytes[3..5] = network_id
        v.extend_from_slice(&network_id.to_be_bytes());
        v.push(0xC0 | 0x01); // version=0, current_next=1
        v.push(0); // section_number
        v.push(0); // last_section_number
                   // bytes[8..10] = reserved(4) | network_descriptors_length(12)
        let net_dll = network_desc.len() as u16;
        v.push(0xF0 | ((net_dll >> 8) as u8 & 0x0F));
        v.push((net_dll & 0xFF) as u8);
        v.extend_from_slice(network_desc);
        // reserved(4) | transport_stream_loop_length(12)
        v.push(0xF0 | ((loop_length >> 8) as u8 & 0x0F));
        v.push((loop_length & 0xFF) as u8);
        for (tsid, onid, desc) in transport_streams {
            v.extend_from_slice(&tsid.to_be_bytes());
            v.extend_from_slice(&onid.to_be_bytes());
            let ts_dll = desc.len() as u16;
            // reserved(4) | transport_descriptors_length(12)
            v.push(0xF0 | ((ts_dll >> 8) as u8 & 0x0F));
            v.push((ts_dll & 0xFF) as u8);
            v.extend_from_slice(desc);
        }
        v.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
        v
    }

    #[test]
    fn parse_actual_and_other_distinguished_by_table_id() {
        let a = build_nit(NitKind::Actual, 0x0001, &[], &[]);
        let o = build_nit(NitKind::Other, 0x0001, &[], &[]);
        assert!(matches!(Nit::parse(&a).unwrap().kind, NitKind::Actual));
        assert!(matches!(Nit::parse(&o).unwrap().kind, NitKind::Other));
    }

    #[test]
    fn parse_network_id_extracted() {
        let bytes = build_nit(NitKind::Actual, 0x0020, &[], &[]);
        let nit = Nit::parse(&bytes).unwrap();
        assert_eq!(nit.network_id, 0x0020);
    }

    #[test]
    fn parse_network_wide_descriptors() {
        let net_desc: [u8; 4] = [0x40, 0x02, 0x4E, 0x65]; // network_name descriptor
        let bytes = build_nit(NitKind::Actual, 0x0001, &net_desc, &[]);
        let nit = Nit::parse(&bytes).unwrap();
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
        let nit = Nit::parse(&bytes).unwrap();
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
        let nit = Nit::parse(&bytes).unwrap();
        assert_eq!(nit.version_number, 0);
        assert!(nit.current_next_indicator);
    }

    #[test]
    fn serialize_round_trip() {
        let net_desc: [u8; 4] = [0x40, 0x02, 0x4E, 0x65];
        let ts_desc: [u8; 3] = [0x43, 0x01, 0x01];
        let nit = Nit {
            kind: NitKind::Actual,
            network_id: 0x0020,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            network_descriptors: DescriptorLoop::new(&net_desc),
            transport_streams: vec![
                NitTransportStream {
                    transport_stream_id: 0x1234,
                    original_network_id: 0x0020,
                    descriptors: DescriptorLoop::new(&ts_desc),
                },
                NitTransportStream {
                    transport_stream_id: 0x5678,
                    original_network_id: 0x0020,
                    descriptors: DescriptorLoop::new(&[]),
                },
            ],
        };
        let mut buf = vec![0u8; nit.serialized_len()];
        nit.serialize_into(&mut buf).unwrap();
        let re = Nit::parse(&buf).unwrap();
        assert_eq!(nit, re);
    }

    #[test]
    fn zero_transport_streams_is_valid() {
        let bytes = build_nit(NitKind::Actual, 0x0001, &[], &[]);
        let nit = Nit::parse(&bytes).unwrap();
        assert_eq!(nit.transport_streams.len(), 0);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Nit::parse(&[0x40, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_nit(NitKind::Actual, 0x0001, &[], &[]);
        bytes[0] = 0x00;
        let err = Nit::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x00, .. }
        ));
    }

    #[test]
    fn serialize_too_small_buffer_returns_error() {
        let nit = Nit {
            kind: NitKind::Actual,
            network_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            network_descriptors: DescriptorLoop::new(&[]),
            transport_streams: vec![],
        };
        let mut buf = vec![0u8; 2];
        let err = nit.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
