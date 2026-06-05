//! Running Status Table — ETSI EN 300 468 §5.2.8.
//!
//! Carried on PID `0x0013` with `table_id = 0x71`. A SHORT-FORM section — there
//! is no version/section header and no CRC. The body is a flat loop of 9-byte
//! entries, each giving the running status of one event:
//!
//! ```text
//! transport_stream_id(16) original_network_id(16) service_id(16)
//! event_id(16) reserved_future_use(5) running_status(3)
//! ```

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for the Running Status Table.
pub const TABLE_ID: u8 = 0x71;
/// Well-known PID on which the RST is carried.
pub const PID: u16 = 0x0013;

const HEADER_LEN: usize = 3;
/// Each entry is 9 bytes: tsid(2) + onid(2) + sid(2) + evid(2) + status(1).
const ENTRY_LEN: usize = 9;

/// One RST entry — the running status of a single event.
///
/// `running_status` is the 3-bit code from EN 300 468 Table 6: 0 undefined,
/// 1 not running, 2 starts in a few seconds, 3 pausing, 4 running,
/// 5 service off-air, 6–7 reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RstEntry {
    /// Transport stream carrying the event.
    pub transport_stream_id: u16,
    /// Originating network.
    pub original_network_id: u16,
    /// Service (matches `program_number` in the PAT).
    pub service_id: u16,
    /// Event identifier.
    pub event_id: u16,
    /// 3-bit running_status code (EN 300 468 Table 6).
    pub running_status: u8,
}

/// Running Status Table (§5.2.8, Table 10).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Rst {
    /// Entries in wire order.
    pub entries: Vec<RstEntry>,
}

impl<'a> Parse<'a> for Rst {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
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
        let section_length = ((bytes[1] & 0x0F) as usize) << 8 | bytes[2] as usize;
        let total = HEADER_LEN + section_length;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len() - HEADER_LEN,
            });
        }
        if section_length % ENTRY_LEN != 0 {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: (section_length / ENTRY_LEN) * ENTRY_LEN,
            });
        }
        let mut entries = Vec::with_capacity(section_length / ENTRY_LEN);
        let mut off = HEADER_LEN;
        while off + ENTRY_LEN <= total {
            entries.push(RstEntry {
                transport_stream_id: u16::from_be_bytes([bytes[off], bytes[off + 1]]),
                original_network_id: u16::from_be_bytes([bytes[off + 2], bytes[off + 3]]),
                service_id: u16::from_be_bytes([bytes[off + 4], bytes[off + 5]]),
                event_id: u16::from_be_bytes([bytes[off + 6], bytes[off + 7]]),
                running_status: bytes[off + 8] & 0x07,
            });
            off += ENTRY_LEN;
        }
        Ok(Rst { entries })
    }
}

impl Serialize for Rst {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.entries.len() * ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length = (len - HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        // section_syntax_indicator=0 (short form), reserved_future_use=1,
        // reserved=11, section_length high nibble.
        buf[1] = 0x70 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        let mut off = HEADER_LEN;
        for e in &self.entries {
            buf[off..off + 2].copy_from_slice(&e.transport_stream_id.to_be_bytes());
            buf[off + 2..off + 4].copy_from_slice(&e.original_network_id.to_be_bytes());
            buf[off + 4..off + 6].copy_from_slice(&e.service_id.to_be_bytes());
            buf[off + 6..off + 8].copy_from_slice(&e.event_id.to_be_bytes());
            // reserved_future_use(5)=1, running_status(3).
            buf[off + 8] = 0xF8 | (e.running_status & 0x07);
            off += ENTRY_LEN;
        }
        Ok(len)
    }
}

impl<'a> Table<'a> for Rst {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Rst {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "RUNNING_STATUS";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_rst(entries: &[RstEntry]) -> Vec<u8> {
        let section_length = (entries.len() * ENTRY_LEN) as u16;
        let mut v = vec![
            TABLE_ID,
            0x70 | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
        ];
        for e in entries {
            v.extend_from_slice(&e.transport_stream_id.to_be_bytes());
            v.extend_from_slice(&e.original_network_id.to_be_bytes());
            v.extend_from_slice(&e.service_id.to_be_bytes());
            v.extend_from_slice(&e.event_id.to_be_bytes());
            v.push(0xF8 | (e.running_status & 0x07));
        }
        v
    }

    fn entry(tsid: u16, onid: u16, sid: u16, evid: u16, rs: u8) -> RstEntry {
        RstEntry {
            transport_stream_id: tsid,
            original_network_id: onid,
            service_id: sid,
            event_id: evid,
            running_status: rs,
        }
    }

    #[test]
    fn parse_empty() {
        let rst = Rst::parse(&build_rst(&[])).unwrap();
        assert!(rst.entries.is_empty());
    }

    #[test]
    fn parse_single_entry() {
        let e = entry(0x1234, 0x0001, 0xABCD, 0x4000, 4);
        let rst = Rst::parse(&build_rst(&[e])).unwrap();
        assert_eq!(rst.entries.len(), 1);
        assert_eq!(rst.entries[0], e);
        assert_eq!(rst.entries[0].running_status, 4); // running
    }

    #[test]
    fn parse_multiple_entries() {
        let es = [
            entry(0x0001, 0x1000, 0x0010, 0x0100, 1),
            entry(0x0002, 0x2000, 0x0020, 0x0200, 4),
            entry(0x0003, 0x3000, 0x0030, 0x0300, 5),
        ];
        let rst = Rst::parse(&build_rst(&es)).unwrap();
        assert_eq!(rst.entries, es);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_rst(&[]);
        bytes[0] = 0x72;
        assert!(matches!(
            Rst::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x72, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            Rst::parse(&[0x71, 0x70]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_non_multiple_loop() {
        // section_length = 4 (not a multiple of 9)
        let bytes = [TABLE_ID, 0x70, 0x04, 0x00, 0x00, 0x00, 0x00];
        assert!(matches!(
            Rst::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let es = [
            entry(0xCAFE, 0xBEEF, 0x1234, 0x5678, 4),
            entry(0x0001, 0x0002, 0x0003, 0x0004, 5),
        ];
        let rst = Rst::parse(&build_rst(&es)).unwrap();
        let mut buf = vec![0u8; rst.serialized_len()];
        rst.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, build_rst(&es));
        assert_eq!(Rst::parse(&buf).unwrap(), rst);
    }

    #[test]
    fn serialize_empty_round_trip() {
        let rst = Rst { entries: vec![] };
        let mut buf = vec![0u8; rst.serialized_len()];
        rst.serialize_into(&mut buf).unwrap();
        assert_eq!(Rst::parse(&buf).unwrap(), rst);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<Rst as Table>::TABLE_ID, 0x71);
        assert_eq!(<Rst as Table>::PID, 0x0013);
    }

    #[test]
    fn serde_json_serializes_fields() {
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let rst = Rst::parse(&build_rst(&[entry(1, 2, 3, 4, 4)])).unwrap();
        let j = serde_json::to_string(&rst).unwrap();
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["entries"][0]["service_id"], 3);
    }
}
