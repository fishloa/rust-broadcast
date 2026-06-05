//! Selection Information Table — ETSI EN 300 468 §7.1.2.
//!
//! Carried on PID 0x001F with table_id 0x7F, only in partial transport streams
//! (e.g. a recording). After the section header it has two loops:
//!   1. `transmission_info_descriptors` — descriptors describing the whole
//!      partial stream, prefixed by a 12-bit length;
//!   2. a per-service loop: `service_id(16) + reserved_future_use(1) +
//!      running_status(3) + service_descriptors_length(12) + descriptors`.
//!
//! Both loops are typed: the transmission-info loop is a [`DescriptorLoop`] and
//! the service loop is a `Vec<SitService>`, each with its own descriptor loop.

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for the Selection Information Table.
pub const TABLE_ID: u8 = 0x7F;
/// Well-known PID on which the SIT is carried.
pub const PID: u16 = 0x001F;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const DESC_LOOP_LEN_FIELD: usize = 2;
const CRC_LEN: usize = 4;
/// Per-service fixed header: service_id(16) + reserved(1)/running_status(3)/
/// service_descriptors_length(12) = 2 + 2 bytes.
const SERVICE_HEADER_LEN: usize = 4;
/// Maximum value of the 12-bit service_descriptors_length field.
const MAX_SERVICE_DESC_LEN: usize = 0x0FFF;

/// One service entry in the SIT service loop (§7.1.2, Table 164).
///
/// Wire layout: `service_id(16) + reserved_future_use(1) + running_status(3) +
/// service_descriptors_length(12) + descriptors`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SitService<'a> {
    /// service_id of this service.
    pub service_id: u16,
    /// 3-bit running_status (0=undefined .. 4=running).
    pub running_status: u8,
    /// Descriptor loop for this service. Serializes as the typed descriptor
    /// sequence; `.raw()` yields the wire bytes.
    pub descriptors: DescriptorLoop<'a>,
}

/// Selection Information Table (§7.1.2, Table 164).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Sit<'a> {
    /// 16-bit field after section_length — reserved_future_use for the SIT
    /// (conventionally 0xFFFF); retained verbatim.
    pub table_id_extension: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// Transmission-info descriptor loop (the first loop). Serializes as the
    /// typed descriptor sequence; `.raw()` yields the wire bytes.
    pub transmission_info_descriptors: DescriptorLoop<'a>,
    /// Per-service loop, in wire order.
    pub services: Vec<SitService<'a>>,
}

impl<'a> Parse<'a> for Sit<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + DESC_LOOP_LEN_FIELD + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Sit",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Sit",
                expected: &[TABLE_ID],
            });
        }
        let section_length = ((bytes[1] & 0x0F) as usize) << 8 | bytes[2] as usize;
        let total = MIN_HEADER_LEN + section_length;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len() - MIN_HEADER_LEN,
            });
        }

        let table_id_extension = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let dl_pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        let ti_len = (((bytes[dl_pos] & 0x0F) as usize) << 8) | bytes[dl_pos + 1] as usize;
        let ti_start = dl_pos + DESC_LOOP_LEN_FIELD;
        let ti_end = ti_start + ti_len;
        let crc_start = total - CRC_LEN;
        if ti_end > crc_start {
            return Err(Error::SectionLengthOverflow {
                declared: ti_len,
                available: crc_start.saturating_sub(ti_start),
            });
        }

        // Everything between the transmission-info loop and the CRC is the
        // per-service loop. Walk it entry by entry, boundary-checked.
        let mut services = Vec::new();
        let mut pos = ti_end;
        while pos < crc_start {
            if pos + SERVICE_HEADER_LEN > crc_start {
                return Err(Error::BufferTooShort {
                    need: pos + SERVICE_HEADER_LEN,
                    have: crc_start,
                    what: "SitService header",
                });
            }
            let service_id = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
            // byte[pos+2]: reserved_future_use(1) | running_status(3) | len_hi(4)
            let running_status = (bytes[pos + 2] >> 4) & 0x07;
            let svc_desc_len = (((bytes[pos + 2] & 0x0F) as usize) << 8) | bytes[pos + 3] as usize;
            let desc_start = pos + SERVICE_HEADER_LEN;
            let desc_end = desc_start + svc_desc_len;
            if desc_end > crc_start {
                return Err(Error::SectionLengthOverflow {
                    declared: svc_desc_len,
                    available: crc_start - desc_start,
                });
            }
            services.push(SitService {
                service_id,
                running_status,
                descriptors: DescriptorLoop::new(&bytes[desc_start..desc_end]),
            });
            pos = desc_end;
        }

        Ok(Sit {
            table_id_extension,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            transmission_info_descriptors: DescriptorLoop::new(&bytes[ti_start..ti_end]),
            services,
        })
    }
}

impl Serialize for Sit<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let svc_bytes: usize = self
            .services
            .iter()
            .map(|s| SERVICE_HEADER_LEN + s.descriptors.len())
            .sum();
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + DESC_LOOP_LEN_FIELD
            + self.transmission_info_descriptors.len()
            + svc_bytes
            + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        // Reject over-range service descriptor loops up front — never truncate.
        // service_descriptors_length is a 12-bit field (max 0x0FFF).
        for svc in &self.services {
            if svc.descriptors.len() > MAX_SERVICE_DESC_LEN {
                return Err(Error::SectionLengthOverflow {
                    declared: svc.descriptors.len(),
                    available: MAX_SERVICE_DESC_LEN,
                });
            }
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length = (len - MIN_HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&self.table_id_extension.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        let dl_pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        let ti_len = self.transmission_info_descriptors.len() as u16;
        buf[dl_pos] = 0xF0 | ((ti_len >> 8) as u8 & 0x0F);
        buf[dl_pos + 1] = (ti_len & 0xFF) as u8;
        let ti_start = dl_pos + DESC_LOOP_LEN_FIELD;
        let ti_end = ti_start + self.transmission_info_descriptors.len();
        buf[ti_start..ti_end].copy_from_slice(self.transmission_info_descriptors.raw());

        let mut pos = ti_end;
        for svc in &self.services {
            buf[pos..pos + 2].copy_from_slice(&svc.service_id.to_be_bytes());
            let dll = svc.descriptors.len() as u16;
            // reserved_future_use(1) emitted as 1 (§5.1 convention) | running_status(3) | len_hi(4)
            buf[pos + 2] = 0x80 | ((svc.running_status & 0x07) << 4) | ((dll >> 8) as u8 & 0x0F);
            buf[pos + 3] = (dll & 0xFF) as u8;
            let desc_start = pos + SERVICE_HEADER_LEN;
            let desc_end = desc_start + svc.descriptors.len();
            buf[desc_start..desc_end].copy_from_slice(svc.descriptors.raw());
            pos = desc_end;
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for Sit<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Sit<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "SELECTION_INFORMATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_sit(
        table_id_extension: u16,
        version: u8,
        ti_desc: &[u8],
        service_loop: &[u8],
    ) -> Vec<u8> {
        let section_length = (EXTENSION_HEADER_LEN
            + DESC_LOOP_LEN_FIELD
            + ti_desc.len()
            + service_loop.len()
            + CRC_LEN) as u16;
        let mut v = vec![
            TABLE_ID,
            0xB0 | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
        ];
        v.extend_from_slice(&table_id_extension.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0x00);
        v.push(0x00);
        let dl = ti_desc.len() as u16;
        v.push(0xF0 | ((dl >> 8) as u8 & 0x0F));
        v.push((dl & 0xFF) as u8);
        v.extend_from_slice(ti_desc);
        v.extend_from_slice(service_loop);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_sit(0x1234, 5, &[], &[]);
        bytes[0] = 0x7E;
        assert!(matches!(
            Sit::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x7E, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            Sit::parse(&[0x7F, 0xB0]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    /// Encode one service entry: service_id + reserved(1)/running_status(3)/
    /// svc_desc_len(12) + descriptor bytes.
    fn service_entry(service_id: u16, running_status: u8, desc: &[u8]) -> Vec<u8> {
        let dll = desc.len() as u16;
        let mut v = Vec::new();
        v.extend_from_slice(&service_id.to_be_bytes());
        v.push(0x80 | ((running_status & 0x07) << 4) | ((dll >> 8) as u8 & 0x0F));
        v.push((dll & 0xFF) as u8);
        v.extend_from_slice(desc);
        v
    }

    #[test]
    fn parse_empty() {
        let bytes = build_sit(0x1234, 5, &[], &[]);
        let sit = Sit::parse(&bytes).unwrap();
        assert_eq!(sit.table_id_extension, 0x1234);
        assert_eq!(sit.version_number, 5);
        assert!(sit.current_next_indicator);
        assert!(sit.transmission_info_descriptors.is_empty());
        assert!(sit.services.is_empty());
    }

    #[test]
    fn parse_two_services_typed() {
        let ti = [0x4D, 0x02, 0x01, 0x02]; // a transmission-info descriptor
        let mut sl = service_entry(0x0001, 4, &[0x48, 0x02, 0xAA, 0xBB]);
        sl.extend(service_entry(0x0002, 2, &[]));
        let bytes = build_sit(0xABCD, 7, &ti, &sl);
        let sit = Sit::parse(&bytes).unwrap();
        assert_eq!(sit.transmission_info_descriptors.raw(), &ti[..]);
        assert_eq!(sit.services.len(), 2);
        assert_eq!(sit.services[0].service_id, 0x0001);
        assert_eq!(sit.services[0].running_status, 4);
        assert_eq!(
            sit.services[0].descriptors.raw(),
            &[0x48, 0x02, 0xAA, 0xBB][..]
        );
        assert_eq!(sit.services[1].service_id, 0x0002);
        assert_eq!(sit.services[1].running_status, 2);
        assert!(sit.services[1].descriptors.is_empty());
    }

    #[test]
    fn parse_rejects_truncated_service_header() {
        // Service loop has only 3 bytes — less than the 4-byte service header.
        let ti = [0x4D, 0x00];
        let bytes = build_sit(0x1234, 0, &ti, &[0x00, 0x01, 0xC0]);
        assert!(matches!(
            Sit::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_inner_descriptor_overflow() {
        // service_descriptors_length declares 5 bytes but only 1 follows.
        let service = [0x00, 0x01, 0x80, 0x05, 0xFF];
        let bytes = build_sit(0x1234, 0, &[], &service);
        assert!(matches!(
            Sit::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn serialize_round_trip_two_services() {
        let ti = [0x4D, 0x02, 0x01, 0x02];
        let sit = Sit {
            table_id_extension: 0xCAFE,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            transmission_info_descriptors: DescriptorLoop::new(&ti),
            services: vec![
                SitService {
                    service_id: 0x0001,
                    running_status: 4,
                    descriptors: DescriptorLoop::new(&[0x48, 0x02, 0xAA, 0xBB]),
                },
                SitService {
                    service_id: 0x0002,
                    running_status: 2,
                    descriptors: DescriptorLoop::new(&[]),
                },
            ],
        };
        let mut buf = vec![0u8; sit.serialized_len()];
        sit.serialize_into(&mut buf).unwrap();
        // Byte-exact: re-parse must equal the original.
        assert_eq!(Sit::parse(&buf).unwrap(), sit);
    }

    #[test]
    fn serialize_round_trip_empty() {
        let bytes = build_sit(0x0001, 0, &[], &[]);
        let sit = Sit::parse(&bytes).unwrap();
        let mut buf = vec![0u8; sit.serialized_len()];
        sit.serialize_into(&mut buf).unwrap();
        assert_eq!(Sit::parse(&buf).unwrap(), sit);
    }

    #[test]
    fn serialize_rejects_over_range_service_desc_len() {
        // A service descriptor loop longer than the 12-bit field can hold.
        let big = vec![0u8; MAX_SERVICE_DESC_LEN + 1];
        let sit = Sit {
            table_id_extension: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            transmission_info_descriptors: DescriptorLoop::new(&[]),
            services: vec![SitService {
                service_id: 0x0001,
                running_status: 4,
                descriptors: DescriptorLoop::new(&big),
            }],
        };
        let mut buf = vec![0u8; sit.serialized_len()];
        assert!(matches!(
            sit.serialize_into(&mut buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<Sit<'_> as Table>::TABLE_ID, 0x7F);
        assert_eq!(<Sit<'_> as Table>::PID, 0x001F);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn sit_serializes_typed_services() {
        // Serialize-only (3.0). The transmission_info loop serializes as a typed
        // descriptor sequence and each service exposes its typed descriptors.
        let mut sl = service_entry(0x0001, 4, &[0x48, 0x02, 0xAA, 0xBB]);
        sl.extend(service_entry(0x0002, 2, &[]));
        let bytes = build_sit(0xDEAD, 9, &[0x4D, 0x00], &sl);
        let sit = Sit::parse(&bytes).unwrap();
        let v = serde_json::to_value(&sit).unwrap();
        assert!(
            v["transmission_info_descriptors"].is_array(),
            "transmission_info_descriptors must serialize as a typed sequence, got {v}"
        );
        assert!(
            v["services"].is_array(),
            "services must serialize as an array, got {v}"
        );
        assert_eq!(v["services"][0]["service_id"], 1);
        assert_eq!(v["services"][0]["running_status"], 4);
        // The service descriptor loop renders as a typed descriptor sequence.
        assert!(
            v["services"][0]["descriptors"].is_array(),
            "service descriptors must serialize as a typed sequence, got {v}"
        );
    }
}
