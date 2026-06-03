//! Service Description Table — ETSI EN 300 468 §5.2.3.
//!
//! SDT lists the services available in a transport stream, with a
//! descriptor loop per service (service_descriptor etc.). The table is
//! split into two variants by table_id: `0x42` for the actual TS the
//! receiver is tuned to, `0x46` for services on other TSes.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id value for SDT describing services on the actual TS.
pub const TABLE_ID_ACTUAL: u8 = 0x42;
/// table_id value for SDT describing services on other TSes.
pub const TABLE_ID_OTHER: u8 = 0x46;
/// Well-known PID on which SDT is carried.
pub const PID: u16 = 0x0011;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
/// Bytes after the extension header and before the first service entry:
/// 2 bytes of original_network_id + 1 reserved_future_use byte.
const POST_EXTENSION_LEN: usize = 3;
const CRC_LEN: usize = 4;
const SERVICE_HEADER_LEN: usize = 5;

/// SDT kind — distinguishes `0x42` (actual) from `0x46` (other).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SdtKind {
    /// Services on the transport stream the receiver is tuned to.
    Actual,
    /// Services on other transport streams (cross-TS SDT).
    Other,
}

/// One service entry in an SDT.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SdtService<'a> {
    /// service_id (matches `program_number` in the PAT).
    pub service_id: u16,
    /// EIT schedule flag — present-and-following events are in the EIT/schedule.
    pub eit_schedule_flag: bool,
    /// EIT P/F flag — present-and-following events are in the EIT present/following.
    pub eit_present_following_flag: bool,
    /// 3-bit running_status (0=undefined .. 4=running).
    pub running_status: u8,
    /// free_CA_mode: `true` = at least one elementary stream is scrambled.
    pub free_ca_mode: bool,
    /// Raw descriptor bytes for this service (descriptor parsing lives elsewhere).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub descriptors: &'a [u8],
}

/// Service Description Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sdt<'a> {
    /// Variant discriminator (table_id 0x42 vs 0x46).
    pub kind: SdtKind,
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Services in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub services: Vec<SdtService<'a>>,
}

impl<'a> Parse<'a> for Sdt<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Sdt",
            });
        }
        let kind = match bytes[0] {
            TABLE_ID_ACTUAL => SdtKind::Actual,
            TABLE_ID_OTHER => SdtKind::Other,
            other => {
                return Err(Error::UnexpectedTableId {
                    table_id: other,
                    what: "Sdt",
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

        let transport_stream_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let original_network_id = u16::from_be_bytes([bytes[8], bytes[9]]);

        let services_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
        let services_end = total - CRC_LEN;
        let mut services = Vec::new();
        let mut pos = services_start;
        while pos + SERVICE_HEADER_LEN <= services_end {
            let service_id = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
            let flags = bytes[pos + 2];
            let eit_schedule_flag = (flags & 0x02) != 0;
            let eit_present_following_flag = (flags & 0x01) != 0;
            let status_and_len_hi = bytes[pos + 3];
            let running_status = (status_and_len_hi >> 5) & 0x07;
            let free_ca_mode = (status_and_len_hi & 0x10) != 0;
            let descriptors_loop_length =
                (((status_and_len_hi & 0x0F) as usize) << 8) | bytes[pos + 4] as usize;
            let desc_start = pos + SERVICE_HEADER_LEN;
            let desc_end = desc_start + descriptors_loop_length;
            if desc_end > services_end {
                return Err(Error::SectionLengthOverflow {
                    declared: descriptors_loop_length,
                    available: services_end - desc_start,
                });
            }
            services.push(SdtService {
                service_id,
                eit_schedule_flag,
                eit_present_following_flag,
                running_status,
                free_ca_mode,
                descriptors: &bytes[desc_start..desc_end],
            });
            pos = desc_end;
        }

        Ok(Sdt {
            kind,
            transport_stream_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            original_network_id,
            services,
        })
    }
}

impl Serialize for Sdt<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let svc_bytes: usize = self
            .services
            .iter()
            .map(|s| SERVICE_HEADER_LEN + s.descriptors.len())
            .sum();
        MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + svc_bytes + CRC_LEN
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
            SdtKind::Actual => TABLE_ID_ACTUAL,
            SdtKind::Other => TABLE_ID_OTHER,
        };
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8..10].copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[10] = 0xFF; // reserved_future_use

        let mut pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
        for svc in &self.services {
            buf[pos..pos + 2].copy_from_slice(&svc.service_id.to_be_bytes());
            let flags = 0xFC
                | (u8::from(svc.eit_schedule_flag) << 1)
                | u8::from(svc.eit_present_following_flag);
            buf[pos + 2] = flags;
            let dll = svc.descriptors.len() as u16;
            buf[pos + 3] = ((svc.running_status & 0x07) << 5)
                | (u8::from(svc.free_ca_mode) << 4)
                | ((dll >> 8) as u8 & 0x0F);
            buf[pos + 4] = (dll & 0xFF) as u8;
            let desc_start = pos + SERVICE_HEADER_LEN;
            buf[desc_start..desc_start + svc.descriptors.len()].copy_from_slice(svc.descriptors);
            pos = desc_start + svc.descriptors.len();
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for Sdt<'a> {
    const TABLE_ID: u8 = TABLE_ID_ACTUAL;
    const PID: u16 = PID;
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestService = (u16, bool, bool, u8, bool, Vec<u8>);

    fn build_sdt(
        kind: SdtKind,
        tsid: u16,
        version: u8,
        original_network_id: u16,
        services: &[TestService],
    ) -> Vec<u8> {
        let svc_bytes: usize = services
            .iter()
            .map(|(_, _, _, _, _, d)| SERVICE_HEADER_LEN + d.len())
            .sum();
        let section_length: u16 =
            (EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + svc_bytes + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(match kind {
            SdtKind::Actual => TABLE_ID_ACTUAL,
            SdtKind::Other => TABLE_ID_OTHER,
        });
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&tsid.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0);
        v.push(0);
        v.extend_from_slice(&original_network_id.to_be_bytes());
        v.push(0xFF);
        for (sid, eit_s, eit_pf, rs, fca, desc) in services {
            v.extend_from_slice(&sid.to_be_bytes());
            let flags = 0xFC | (u8::from(*eit_s) << 1) | u8::from(*eit_pf);
            v.push(flags);
            let dll = desc.len() as u16;
            v.push(((*rs & 0x07) << 5) | (u8::from(*fca) << 4) | ((dll >> 8) as u8 & 0x0F));
            v.push((dll & 0xFF) as u8);
            v.extend_from_slice(desc);
        }
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_actual_and_other_tables_distinguished_by_table_id() {
        let a = build_sdt(SdtKind::Actual, 1, 0, 0x20, &[]);
        let o = build_sdt(SdtKind::Other, 1, 0, 0x20, &[]);
        assert!(matches!(Sdt::parse(&a).unwrap().kind, SdtKind::Actual));
        assert!(matches!(Sdt::parse(&o).unwrap().kind, SdtKind::Other));
    }

    #[test]
    fn parse_services_with_descriptor_bytes() {
        let bytes = build_sdt(
            SdtKind::Actual,
            1,
            0,
            0x20,
            &[(100, true, true, 4, false, vec![0x48, 0x05, 0x01, 0x02, 0x03, 0x04, 0x05])],
        );
        let sdt = Sdt::parse(&bytes).unwrap();
        assert_eq!(sdt.services.len(), 1);
        assert_eq!(sdt.services[0].service_id, 100);
        assert!(sdt.services[0].eit_schedule_flag);
        assert!(sdt.services[0].eit_present_following_flag);
        assert_eq!(sdt.services[0].running_status, 4);
        assert!(!sdt.services[0].free_ca_mode);
        assert_eq!(
            sdt.services[0].descriptors,
            &[0x48, 0x05, 0x01, 0x02, 0x03, 0x04, 0x05][..]
        );
    }

    #[test]
    fn service_free_ca_mode_flag_extracted() {
        let bytes = build_sdt(
            SdtKind::Actual,
            1,
            0,
            0x20,
            &[(1, false, false, 0, true, vec![])],
        );
        let sdt = Sdt::parse(&bytes).unwrap();
        assert!(sdt.services[0].free_ca_mode);
    }

    #[test]
    fn service_running_status_extracted() {
        let bytes = build_sdt(
            SdtKind::Actual,
            1,
            0,
            0x20,
            &[(1, false, false, 2, false, vec![])],
        );
        let sdt = Sdt::parse(&bytes).unwrap();
        assert_eq!(sdt.services[0].running_status, 2);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Sdt::parse(&[0x42, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_sdt(SdtKind::Actual, 1, 0, 0x20, &[]);
        bytes[0] = 0x00;
        let err = Sdt::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::UnexpectedTableId { table_id: 0x00, .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let desc1: [u8; 4] = [0x48, 0x02, 0xAA, 0xBB];
        let sdt = Sdt {
            kind: SdtKind::Actual,
            transport_stream_id: 0x1234,
            version_number: 5,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            original_network_id: 0x0020,
            services: vec![
                SdtService {
                    service_id: 100,
                    eit_schedule_flag: true,
                    eit_present_following_flag: false,
                    running_status: 4,
                    free_ca_mode: false,
                    descriptors: &desc1,
                },
                SdtService {
                    service_id: 101,
                    eit_schedule_flag: false,
                    eit_present_following_flag: true,
                    running_status: 2,
                    free_ca_mode: true,
                    descriptors: &[],
                },
            ],
        };
        let mut buf = vec![0u8; sdt.serialized_len()];
        sdt.serialize_into(&mut buf).unwrap();
        let re = Sdt::parse(&buf).unwrap();
        assert_eq!(sdt, re);
    }

    #[test]
    fn zero_services_is_valid() {
        let bytes = build_sdt(SdtKind::Actual, 1, 0, 0x20, &[]);
        let sdt = Sdt::parse(&bytes).unwrap();
        assert_eq!(sdt.services.len(), 0);
    }
}
