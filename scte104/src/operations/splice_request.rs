//! splice_request_data() — ANSI/SCTE 104 2023 §9.3.1, Table 9-5 (opID 0x0101).
//!
//! The primary Normal-usage request for initiating SCTE 35 splice insertion.
//! Can be elaborated by Supplemental requests (descriptor inserts, encryption,
//! component mode).

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use dvb_common::{Parse, Serialize};

/// `opID` for splice_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0101;

/// Fixed wire length of `splice_request_data()`.
pub const LEN: usize = 15;
// splice_insert_type(1) + splice_event_id(4) + unique_program_id(2) +
// pre_roll_time(2) + break_duration(2) + avail_num(1) + avails_expected(1) +
// auto_return_flag(1) + not_an_entry_flag(1)

/// `splice_insert_type` values — §9.3.1, Table 9-6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SpliceInsertType {
    /// Reserved.
    Reserved,
    /// spliceStart_normal — at least one section before the splice point.
    SpliceStartNormal,
    /// spliceStart_immediate — at the splice point.
    SpliceStartImmediate,
    /// spliceEnd_normal — terminate a splice.
    SpliceEndNormal,
    /// spliceEnd_immediate — terminate before the splice point.
    SpliceEndImmediate,
    /// splice_cancel — cancel a recently sent spliceStart_normal.
    SpliceCancel,
    /// Unknown value.
    Unknown(u8),
}

impl SpliceInsertType {
    /// Parse from a wire byte.
    #[must_use]
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Reserved,
            1 => Self::SpliceStartNormal,
            2 => Self::SpliceStartImmediate,
            3 => Self::SpliceEndNormal,
            4 => Self::SpliceEndImmediate,
            5 => Self::SpliceCancel,
            _ => Self::Unknown(b),
        }
    }

    /// Wire byte.
    #[must_use]
    pub fn to_byte(self) -> u8 {
        match self {
            Self::Reserved => 0,
            Self::SpliceStartNormal => 1,
            Self::SpliceStartImmediate => 2,
            Self::SpliceEndNormal => 3,
            Self::SpliceEndImmediate => 4,
            Self::SpliceCancel => 5,
            Self::Unknown(b) => b,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::SpliceStartNormal => "splice start normal",
            Self::SpliceStartImmediate => "splice start immediate",
            Self::SpliceEndNormal => "splice end normal",
            Self::SpliceEndImmediate => "splice end immediate",
            Self::SpliceCancel => "splice cancel",
            Self::Unknown(_) => "unknown",
        }
    }
}

dvb_common::impl_spec_display!(SpliceInsertType, Unknown);

/// splice_request_data() — §9.3.1, Table 9-5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceRequest {
    /// `splice_insert_type` — 1 byte.
    pub splice_insert_type: SpliceInsertType,
    /// `splice_event_id` — 4 bytes.
    pub splice_event_id: u32,
    /// `unique_program_id` — 2 bytes.
    pub unique_program_id: u16,
    /// `pre_roll_time` — 2 bytes, milliseconds.
    pub pre_roll_time: u16,
    /// `break_duration` — 2 bytes, tenths of seconds.
    pub break_duration: u16,
    /// `avail_num` — 1 byte.
    pub avail_num: u8,
    /// `avails_expected` — 1 byte.
    pub avails_expected: u8,
    /// `auto_return_flag` — 1 byte (0 or non-zero).
    pub auto_return_flag: u8,
    /// `not_an_entry_flag` — 1 byte (0 or non-zero).
    pub not_an_entry_flag: u8,
}

impl Default for SpliceRequest {
    fn default() -> Self {
        Self {
            splice_insert_type: SpliceInsertType::Reserved,
            splice_event_id: 0,
            unique_program_id: 0,
            pre_roll_time: 0,
            break_duration: 0,
            avail_num: 0,
            avails_expected: 0,
            auto_return_flag: 0,
            not_an_entry_flag: 0,
        }
    }
}

impl<'a> Parse<'a> for SpliceRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < LEN {
            return Err(Error::BufferTooShort {
                need: LEN,
                have: bytes.len(),
                what: "splice_request_data",
            });
        }
        Ok(Self {
            splice_insert_type: SpliceInsertType::from_byte(bytes[0]),
            splice_event_id: u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
            unique_program_id: u16::from_be_bytes([bytes[5], bytes[6]]),
            pre_roll_time: u16::from_be_bytes([bytes[7], bytes[8]]),
            break_duration: u16::from_be_bytes([bytes[9], bytes[10]]),
            avail_num: bytes[11],
            avails_expected: bytes[12],
            auto_return_flag: bytes[13],
            not_an_entry_flag: bytes[14],
        })
    }
}

impl Serialize for SpliceRequest {
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
        buf[0] = self.splice_insert_type.to_byte();
        buf[1..5].copy_from_slice(&self.splice_event_id.to_be_bytes());
        buf[5..7].copy_from_slice(&self.unique_program_id.to_be_bytes());
        buf[7..9].copy_from_slice(&self.pre_roll_time.to_be_bytes());
        buf[9..11].copy_from_slice(&self.break_duration.to_be_bytes());
        buf[11] = self.avail_num;
        buf[12] = self.avails_expected;
        buf[13] = self.auto_return_flag;
        buf[14] = self.not_an_entry_flag;
        Ok(LEN)
    }
}

impl<'a> OperationDef<'a> for SpliceRequest {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "SPLICE_REQUEST";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let sr = SpliceRequest {
            splice_insert_type: SpliceInsertType::SpliceStartNormal,
            splice_event_id: 0x0000_0042,
            unique_program_id: 1,
            pre_roll_time: 5000,
            break_duration: 300,
            avail_num: 0,
            avails_expected: 0,
            auto_return_flag: 1,
            not_an_entry_flag: 0,
        };
        let bytes = sr.to_bytes();
        assert_eq!(bytes.len(), LEN);
        let back = SpliceRequest::parse(&bytes).unwrap();
        assert_eq!(sr, back);
        let b2 = back.to_bytes();
        assert_eq!(bytes, b2);
    }

    #[test]
    fn mutate_field_changes_output() {
        let sr = SpliceRequest {
            splice_insert_type: SpliceInsertType::SpliceStartNormal,
            splice_event_id: 1,
            unique_program_id: 1,
            pre_roll_time: 1000,
            break_duration: 300,
            avail_num: 0,
            avails_expected: 0,
            auto_return_flag: 0,
            not_an_entry_flag: 0,
        };
        let bytes = sr.to_bytes();
        let mut sr2 = sr;
        sr2.splice_event_id = 999;
        assert_ne!(sr2.to_bytes(), bytes);
    }

    #[test]
    fn splice_insert_type_name() {
        assert_eq!(
            SpliceInsertType::SpliceStartNormal.name(),
            "splice start normal"
        );
        assert_eq!(SpliceInsertType::Reserved.name(), "reserved");
    }
}
