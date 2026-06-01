//! Running Status Table — ETSI EN 300 468 §5.2.7.
//!
//! RST is carried on PID 0x0013 with table_id 0x71. It provides the
//! running status of services in a transport stream.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for Running Status Table.
pub const TABLE_ID: u8 = 0x71;
/// Well-known PID on which RST is carried.
pub const PID: u16 = 0x0013;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const CRC_LEN: usize = 4;
/// Each service entry: reserved(4) + running_status(2) + service_id(16).
const SERVICE_ENTRY_LEN: usize = 3;

/// Running status value for a single service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RunningStatus {
    /// Service is not defined.
    NotDefined,
    /// Service has started but is not yet running (e.g. waiting for a scheduled start).
    Start,
    /// Service has ended.
    End,
    /// Service is running normally.
    Running,
}

impl RunningStatus {
    fn from_bits(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::NotDefined),
            1 => Some(Self::Start),
            2 => Some(Self::End),
            3 => Some(Self::Running),
            _ => None,
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::NotDefined => 0,
            Self::Start => 1,
            Self::End => 2,
            Self::Running => 3,
        }
    }
}

/// One service entry in an RST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RstService {
    /// 2-bit running_status (0=not defined, 1=start, 2=end, 3=running).
    pub running_status: RunningStatus,
    /// service_id (matches `program_number` in the PAT).
    pub service_id: u16,
}

/// Running Status Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rst {
    /// transport_stream_id (16-bit table_id_extension).
    pub transport_stream_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// Service entries in wire order.
    pub services: Vec<RstService>,
}

impl<'a> Parse<'a> for Rst {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Rst",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Rst",
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

        let transport_stream_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        // Service loop starts after the extension header and ends before CRC.
        let loop_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        let loop_end = total - CRC_LEN;
        let loop_len = loop_end - loop_start;

        if loop_len % SERVICE_ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TABLE_ID,
                reason: "RST service loop length not a multiple of 3",
            });
        }

        let mut services = Vec::with_capacity(loop_len / SERVICE_ENTRY_LEN);
        let mut offset = loop_start;
        while offset < loop_end {
            let status_bits = bytes[offset] & 0x03;
            let running_status = RunningStatus::from_bits(status_bits).ok_or(Error::InvalidDescriptor {
                tag: TABLE_ID,
                reason: "RST running_status value out of range",
            })?;
            let service_id = u16::from_be_bytes([bytes[offset + 1], bytes[offset + 2]]);
            services.push(RstService {
                running_status,
                service_id,
            });
            offset += SERVICE_ENTRY_LEN;
        }

        Ok(Rst {
            transport_stream_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            services,
        })
    }
}

impl Serialize for Rst {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + self.services.len() * SERVICE_ENTRY_LEN
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
        buf[3..5].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        let mut offset = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        for svc in &self.services {
            buf[offset] = 0xFC | (svc.running_status.to_bits() & 0x03);
            buf[offset + 1..offset + 3].copy_from_slice(&svc.service_id.to_be_bytes());
            offset += SERVICE_ENTRY_LEN;
        }

        let crc_pos = len - CRC_LEN;
        buf[crc_pos..len].copy_from_slice(&[0, 0, 0, 0]);

        Ok(len)
    }
}

impl<'a> Table<'a> for Rst {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_rst(transport_stream_id: u16, version: u8, services: &[(RunningStatus, u16)]) -> Vec<u8> {
        let loop_len = services.len() * SERVICE_ENTRY_LEN;
        let section_length: u16 = (EXTENSION_HEADER_LEN + loop_len + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID);
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&transport_stream_id.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0x00);
        v.push(0x00);
        for (status, service_id) in services {
            v.push(0xFC | status.to_bits());
            v.extend_from_slice(&service_id.to_be_bytes());
        }
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_rst(0x1234, 5, &[]);
        bytes[0] = 0x72;
        assert!(matches!(
            Rst::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x72, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Rst::parse(&[0x71, 0xB0]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_empty_service_loop() {
        let bytes = build_rst(0x1234, 5, &[]);
        let rst = Rst::parse(&bytes).unwrap();
        assert_eq!(rst.transport_stream_id, 0x1234);
        assert_eq!(rst.version_number, 5);
        assert!(rst.current_next_indicator);
        assert_eq!(rst.section_number, 0);
        assert_eq!(rst.last_section_number, 0);
        assert!(rst.services.is_empty());
    }

    #[test]
    fn parse_single_service() {
        let bytes = build_rst(0xABCD, 7, &[(RunningStatus::Running, 0x0001)]);
        let rst = Rst::parse(&bytes).unwrap();
        assert_eq!(rst.transport_stream_id, 0xABCD);
        assert_eq!(rst.version_number, 7);
        assert_eq!(rst.services.len(), 1);
        assert_eq!(rst.services[0].running_status, RunningStatus::Running);
        assert_eq!(rst.services[0].service_id, 0x0001);
    }

    #[test]
    fn parse_multiple_services() {
        let bytes = build_rst(0x5678, 3, &[
            (RunningStatus::NotDefined, 0x1000),
            (RunningStatus::Start, 0x2000),
            (RunningStatus::End, 0x3000),
            (RunningStatus::Running, 0x4000),
        ]);
        let rst = Rst::parse(&bytes).unwrap();
        assert_eq!(rst.services.len(), 4);
        assert_eq!(rst.services[0].running_status, RunningStatus::NotDefined);
        assert_eq!(rst.services[1].running_status, RunningStatus::Start);
        assert_eq!(rst.services[2].running_status, RunningStatus::End);
        assert_eq!(rst.services[3].running_status, RunningStatus::Running);
    }

    #[test]
    fn parse_rejects_malformed_loop_length() {
        // Manually construct a section where loop_len is not a multiple of 3.
        // section_length = 10 → total = 13, loop area = bytes[8..9] (1 byte, not %3).
        let bytes: [u8; 13] = [
            TABLE_ID, // table_id
            0xB0,     // SSI=1, reserved=11, section_length hi = 0
            0x0A,     // section_length lo = 10 → total = 13
            0x12, 0x34, // table_id_extension
            0xC1,     // reserved=11, version=0, current_next=1
            0x00,     // section_number
            0x00,     // last_section_number
            0xFF,     // 1 byte of "loop" — not a multiple of SERVICE_ENTRY_LEN
            0x00, 0x00, 0x00, 0x00, // CRC
        ];
        let err = Rst::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let services = [
            (RunningStatus::NotDefined, 0x1000),
            (RunningStatus::Start, 0x2000),
            (RunningStatus::End, 0x3000),
            (RunningStatus::Running, 0x4000),
        ];
        let bytes = build_rst(0xCAFE, 3, &services);
        let rst = Rst::parse(&bytes).unwrap();
        let mut buf = vec![0u8; rst.serialized_len()];
        rst.serialize_into(&mut buf).unwrap();
        let re = Rst::parse(&buf).unwrap();
        assert_eq!(rst, re);
    }

    #[test]
    fn serialize_round_trip_empty() {
        let bytes = build_rst(0x0001, 0, &[]);
        let rst = Rst::parse(&bytes).unwrap();
        let mut buf = vec![0u8; rst.serialized_len()];
        rst.serialize_into(&mut buf).unwrap();
        let re = Rst::parse(&buf).unwrap();
        assert_eq!(rst, re);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<Rst as Table>::TABLE_ID, 0x71);
        assert_eq!(<Rst as Table>::PID, 0x0013);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn rst_round_trips_via_json() {
        let services = [
            (RunningStatus::NotDefined, 0x1000),
            (RunningStatus::Start, 0x2000),
            (RunningStatus::End, 0x3000),
            (RunningStatus::Running, 0x4000),
        ];
        let bytes = build_rst(0xDEAD, 9, &services);
        let rst = Rst::parse(&bytes).unwrap();
        let j = serde_json::to_string(&rst).unwrap();
        let back: Rst = serde_json::from_str(&j).unwrap();
        assert_eq!(rst, back);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn rst_empty_round_trips_via_json() {
        let bytes = build_rst(0x0001, 0, &[]);
        let rst = Rst::parse(&bytes).unwrap();
        let j = serde_json::to_string(&rst).unwrap();
        let back: Rst = serde_json::from_str(&j).unwrap();
        assert_eq!(rst, back);
    }

    #[test]
    fn running_status_from_bits_all_values() {
        assert_eq!(RunningStatus::from_bits(0), Some(RunningStatus::NotDefined));
        assert_eq!(RunningStatus::from_bits(1), Some(RunningStatus::Start));
        assert_eq!(RunningStatus::from_bits(2), Some(RunningStatus::End));
        assert_eq!(RunningStatus::from_bits(3), Some(RunningStatus::Running));
        assert_eq!(RunningStatus::from_bits(4), None);
    }

    #[test]
    fn running_status_to_bits_round_trip() {
        for status in [
            RunningStatus::NotDefined,
            RunningStatus::Start,
            RunningStatus::End,
            RunningStatus::Running,
        ] {
            assert_eq!(RunningStatus::from_bits(status.to_bits()), Some(status));
        }
    }


    #[test]
    fn parse_section_number_and_last_section_number() {
        let bytes = build_rst(0x1234, 5, &[]);
        let rst = Rst::parse(&bytes).unwrap();
        assert_eq!(rst.section_number, 0);
        assert_eq!(rst.last_section_number, 0);
    }

    #[test]
    fn parse_current_next_indicator_false() {
        let mut bytes = build_rst(0x1234, 5, &[]);
        bytes[5] &= !0x01; // Clear current_next_indicator bit
        let rst = Rst::parse(&bytes).unwrap();
        assert!(!rst.current_next_indicator);
    }

    #[test]
    fn parse_non_zero_section_numbers() {
        let mut bytes = build_rst(0x1234, 5, &[]);
        bytes[6] = 1; // section_number
        bytes[7] = 2; // last_section_number
        let rst = Rst::parse(&bytes).unwrap();
        assert_eq!(rst.section_number, 1);
        assert_eq!(rst.last_section_number, 2);
    }
}
