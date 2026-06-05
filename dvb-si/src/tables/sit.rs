//! Selection Information Table — ETSI EN 300 468 §7.1.2.
//!
//! Carried on PID 0x001F with table_id 0x7F, only in partial transport streams
//! (e.g. a recording). After the section header it has two loops:
//!   1. `transmission_info_descriptors` — descriptors describing the whole
//!      partial stream, prefixed by a 12-bit length;
//!   2. a per-service loop: `service_id(16) + reserved(1) + running_status(3) +
//!      service_descriptors_length(12) + descriptors`.
//!
//! Both loops are exposed as raw bytes for the caller to walk with the
//! descriptor parsers.

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

/// Selection Information Table (§7.1.2, Table 164).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sit {
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
    /// Transmission-info descriptor loop bytes (the first loop).
    pub transmission_info_descriptors: Vec<u8>,
    /// Per-service loop bytes (service_id + running_status + descriptors), raw.
    pub service_loop: Vec<u8>,
}

impl<'a> Parse<'a> for Sit {
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

        Ok(Sit {
            table_id_extension,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            transmission_info_descriptors: bytes[ti_start..ti_end].to_vec(),
            // Everything between the transmission-info loop and the CRC is the
            // per-service loop, kept raw.
            service_loop: bytes[ti_end..crc_start].to_vec(),
        })
    }
}

impl Serialize for Sit {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + DESC_LOOP_LEN_FIELD
            + self.transmission_info_descriptors.len()
            + self.service_loop.len()
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
        buf[ti_start..ti_end].copy_from_slice(&self.transmission_info_descriptors);
        let sl_end = ti_end + self.service_loop.len();
        buf[ti_end..sl_end].copy_from_slice(&self.service_loop);

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for Sit {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Sit {
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

    #[test]
    fn parse_empty() {
        let sit = Sit::parse(&build_sit(0x1234, 5, &[], &[])).unwrap();
        assert_eq!(sit.table_id_extension, 0x1234);
        assert_eq!(sit.version_number, 5);
        assert!(sit.current_next_indicator);
        assert!(sit.transmission_info_descriptors.is_empty());
        assert!(sit.service_loop.is_empty());
    }

    #[test]
    fn parse_separates_both_loops() {
        let ti = [0x4D, 0x02, 0x01, 0x02]; // a transmission-info descriptor
                                           // one service entry: service_id=0x0001, running_status=4, svc_desc_len=0
        let service = [0x00, 0x01, 0x80 | (4 << 4), 0x00];
        let sit = Sit::parse(&build_sit(0xABCD, 7, &ti, &service)).unwrap();
        assert_eq!(sit.transmission_info_descriptors, &ti[..]);
        assert_eq!(sit.service_loop, &service[..]);
    }

    #[test]
    fn serialize_round_trip() {
        let ti = [0x4D, 0x02, 0x01, 0x02];
        let service = [0x00, 0x01, 0xC0, 0x00];
        let sit = Sit::parse(&build_sit(0xCAFE, 3, &ti, &service)).unwrap();
        let mut buf = vec![0u8; sit.serialized_len()];
        sit.serialize_into(&mut buf).unwrap();
        assert_eq!(Sit::parse(&buf).unwrap(), sit);
    }

    #[test]
    fn serialize_round_trip_empty() {
        let sit = Sit::parse(&build_sit(0x0001, 0, &[], &[])).unwrap();
        let mut buf = vec![0u8; sit.serialized_len()];
        sit.serialize_into(&mut buf).unwrap();
        assert_eq!(Sit::parse(&buf).unwrap(), sit);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<Sit as Table>::TABLE_ID, 0x7F);
        assert_eq!(<Sit as Table>::PID, 0x001F);
    }

    #[test]
    fn sit_round_trips_via_json() {
        let sit = Sit::parse(&build_sit(
            0xDEAD,
            9,
            &[0x4D, 0x00],
            &[0x00, 0x01, 0xC0, 0x00],
        ))
        .unwrap();
        let j = serde_json::to_string(&sit).unwrap();
        assert_eq!(serde_json::from_str::<Sit>(&j).unwrap(), sit);
    }
}
