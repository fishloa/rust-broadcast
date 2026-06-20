//! insert_time_descriptor() — ANSI/SCTE 104 2023 §9.8.10, Table 9-32 (opID 0x0110).
//!
//! Supplemental usage. Adds a time_descriptor with PTP/TAI time to the
//! resulting SCTE 35 section.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use dvb_common::{Parse, Serialize};

/// `opID` for insert_time_descriptor (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0110;

/// Fixed wire length.
pub const LEN: usize = 12;

/// insert_time_descriptor() — §9.8.10, Table 9-32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertTimeDescriptor {
    /// `TAI_seconds` — 6 bytes (48-bit big-endian).
    pub tai_seconds: u64,
    /// `TAI_ns` — 4 bytes (32-bit big-endian).
    pub tai_ns: u32,
    /// `UTC_offset` — 2 bytes (16-bit big-endian).
    pub utc_offset: u16,
}

impl<'a> Parse<'a> for InsertTimeDescriptor {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < LEN {
            return Err(Error::BufferTooShort {
                need: LEN,
                have: bytes.len(),
                what: "insert_time_descriptor",
            });
        }
        // TAI_seconds: 6 bytes (48-bit)
        let tai_seconds = (u64::from(bytes[0]) << 40)
            | (u64::from(bytes[1]) << 32)
            | (u64::from(bytes[2]) << 24)
            | (u64::from(bytes[3]) << 16)
            | (u64::from(bytes[4]) << 8)
            | u64::from(bytes[5]);
        let tai_ns = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        let utc_offset = u16::from_be_bytes([bytes[10], bytes[11]]);
        Ok(Self {
            tai_seconds,
            tai_ns,
            utc_offset,
        })
    }
}

impl Serialize for InsertTimeDescriptor {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < LEN {
            return Err(Error::OutputBufferTooSmall {
                need: LEN,
                have: buf.len(),
            });
        }
        // TAI_seconds: 6 bytes (48-bit)
        buf[0] = (self.tai_seconds >> 40) as u8;
        buf[1] = (self.tai_seconds >> 32) as u8;
        buf[2] = (self.tai_seconds >> 24) as u8;
        buf[3] = (self.tai_seconds >> 16) as u8;
        buf[4] = (self.tai_seconds >> 8) as u8;
        buf[5] = self.tai_seconds as u8;
        buf[6..10].copy_from_slice(&self.tai_ns.to_be_bytes());
        buf[10..12].copy_from_slice(&self.utc_offset.to_be_bytes());
        Ok(LEN)
    }
}

impl<'a> OperationDef<'a> for InsertTimeDescriptor {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_TIME_DESCRIPTOR";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = InsertTimeDescriptor {
            tai_seconds: 0x0000_1234_5678_9ABC,
            tai_ns: 500_000_000,
            utc_offset: 37,
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), LEN);
        let back = InsertTimeDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InsertTimeDescriptor::default();
        let bytes = op.to_bytes();
        let mut op2 = op;
        op2.utc_offset = 99;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
