//! splice_request_data() — ANSI/SCTE 104 2023 §9.3.1, Table 9-5 (opID 0x0101).
//!
//! The primary Normal-usage request for initiating SCTE 35 splice insertion.
//! Can be elaborated by Supplemental requests (descriptor inserts, encryption,
//! component mode).

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// `opID` for splice_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0101;

/// Fixed wire length of `splice_request_data()`.
pub const LEN: usize = 15;
// Field byte offsets within `splice_request_data()` (ANSI/SCTE 104 2023 §9.3.2).
const SPLICE_INSERT_TYPE_OFFSET: usize = 0;
const SPLICE_EVENT_ID_OFFSET: usize = 1;
const UNIQUE_PROGRAM_ID_OFFSET: usize = 5;
const PRE_ROLL_TIME_OFFSET: usize = 7;
const BREAK_DURATION_OFFSET: usize = 9;
const AVAIL_NUM_OFFSET: usize = 11;
const AVAILS_EXPECTED_OFFSET: usize = 12;
const AUTO_RETURN_FLAG_OFFSET: usize = 13;
const NOT_AN_ENTRY_FLAG_OFFSET: usize = 14;
/// Offset of the second 16-bit word within a 4-byte big-endian word.
const U32_SECOND_WORD_OFFSET: usize = core::mem::size_of::<u16>();
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

broadcast_common::impl_spec_display!(SpliceInsertType, Unknown);

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
        need_in(bytes, LEN, "splice_request_data")?;
        Ok(Self {
            splice_insert_type: SpliceInsertType::from_byte(bytes[SPLICE_INSERT_TYPE_OFFSET]),
            splice_event_id: u32::from_be_bytes([
                bytes[SPLICE_EVENT_ID_OFFSET],
                bytes[SPLICE_EVENT_ID_OFFSET + 1],
                bytes[SPLICE_EVENT_ID_OFFSET + U32_SECOND_WORD_OFFSET],
                bytes[SPLICE_EVENT_ID_OFFSET + U32_SECOND_WORD_OFFSET + 1],
            ]),
            unique_program_id: u16::from_be_bytes([
                bytes[UNIQUE_PROGRAM_ID_OFFSET],
                bytes[UNIQUE_PROGRAM_ID_OFFSET + 1],
            ]),
            pre_roll_time: u16::from_be_bytes([
                bytes[PRE_ROLL_TIME_OFFSET],
                bytes[PRE_ROLL_TIME_OFFSET + 1],
            ]),
            break_duration: u16::from_be_bytes([
                bytes[BREAK_DURATION_OFFSET],
                bytes[BREAK_DURATION_OFFSET + 1],
            ]),
            avail_num: bytes[AVAIL_NUM_OFFSET],
            avails_expected: bytes[AVAILS_EXPECTED_OFFSET],
            auto_return_flag: bytes[AUTO_RETURN_FLAG_OFFSET],
            not_an_entry_flag: bytes[NOT_AN_ENTRY_FLAG_OFFSET],
        })
    }
}

impl Serialize for SpliceRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, LEN)?;
        buf[SPLICE_INSERT_TYPE_OFFSET] = self.splice_insert_type.to_byte();
        buf[1..5].copy_from_slice(&self.splice_event_id.to_be_bytes());
        buf[5..7].copy_from_slice(&self.unique_program_id.to_be_bytes());
        buf[7..9].copy_from_slice(&self.pre_roll_time.to_be_bytes());
        buf[9..11].copy_from_slice(&self.break_duration.to_be_bytes());
        buf[AVAIL_NUM_OFFSET] = self.avail_num;
        buf[AVAILS_EXPECTED_OFFSET] = self.avails_expected;
        buf[AUTO_RETURN_FLAG_OFFSET] = self.auto_return_flag;
        buf[NOT_AN_ENTRY_FLAG_OFFSET] = self.not_an_entry_flag;
        Ok(LEN)
    }
}

impl OperationDef<'_> for SpliceRequest {
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
        let bytes = sr.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), LEN);
        let back = SpliceRequest::parse(&bytes).unwrap();
        assert_eq!(sr, back);
        let b2 = back.try_to_bytes().unwrap();
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
        let bytes = sr.try_to_bytes().unwrap();
        let mut sr2 = sr;
        sr2.splice_event_id = 999;
        assert_ne!(sr2.try_to_bytes().unwrap(), bytes);
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
