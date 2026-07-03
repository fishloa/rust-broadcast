//! schedule_definition_data() — ANSI/SCTE 104 2023 §9.7.2, Table 9-19 (opID 0x010E).
//!
//! Supplemental request following start_schedule_download. Defines one
//! avail/splice event in the schedule.

use crate::error::{Error, Result};
use crate::time::SpliceScheduleTime;
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for schedule_definition_data (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x010E;

/// Fixed wire length of `schedule_definition_data()`.
pub const LEN: usize = 16;
// splice_schedule_command(1) + splice_event_id(4) + time(4) +
// unique_program_id(2) + auto_return(1) + break_duration(2) +
// avail_num(1) + avails_expected(1)

/// `splice_schedule_command` values — §9.7.2, Table 9-20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SpliceScheduleCommand {
    /// Reserved.
    Reserved,
    /// splice_insert — away from network.
    SpliceInsert,
    /// splice_return — back to network.
    SpliceReturn,
    /// splice_cancel — cancel a scheduled event.
    SpliceCancel,
    /// Unknown value.
    Unknown(u8),
}

impl SpliceScheduleCommand {
    /// Parse from a wire byte.
    #[must_use]
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Reserved,
            1 => Self::SpliceInsert,
            3 => Self::SpliceReturn,
            5 => Self::SpliceCancel,
            _ => Self::Unknown(b),
        }
    }

    /// Wire byte.
    #[must_use]
    pub fn to_byte(self) -> u8 {
        match self {
            Self::Reserved => 0,
            Self::SpliceInsert => 1,
            Self::SpliceReturn => 3,
            Self::SpliceCancel => 5,
            Self::Unknown(b) => b,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::SpliceInsert => "splice insert",
            Self::SpliceReturn => "splice return",
            Self::SpliceCancel => "splice cancel",
            Self::Unknown(_) => "unknown",
        }
    }
}

broadcast_common::impl_spec_display!(SpliceScheduleCommand, Unknown);

/// schedule_definition_data() — §9.7.2, Table 9-19.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ScheduleDefinition {
    /// `splice_schedule_command` — 1 byte.
    pub splice_schedule_command: SpliceScheduleCommand,
    /// `splice_event_id` — 4 bytes.
    pub splice_event_id: u32,
    /// `time()` — 4 bytes (seconds since GPS epoch).
    pub time: SpliceScheduleTime,
    /// `unique_program_id` — 2 bytes.
    pub unique_program_id: u16,
    /// `auto_return` — 1 byte.
    pub auto_return: u8,
    /// `break_duration` — 2 bytes, tenths of seconds.
    pub break_duration: u16,
    /// `avail_num` — 1 byte.
    pub avail_num: u8,
    /// `avails_expected` — 1 byte.
    pub avails_expected: u8,
}

impl Default for ScheduleDefinition {
    fn default() -> Self {
        Self {
            splice_schedule_command: SpliceScheduleCommand::Reserved,
            splice_event_id: 0,
            time: SpliceScheduleTime { seconds: 0 },
            unique_program_id: 0,
            auto_return: 0,
            break_duration: 0,
            avail_num: 0,
            avails_expected: 0,
        }
    }
}

impl<'a> Parse<'a> for ScheduleDefinition {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < LEN {
            return Err(Error::BufferTooShort {
                need: LEN,
                have: bytes.len(),
                what: "schedule_definition_data",
            });
        }
        Ok(Self {
            splice_schedule_command: SpliceScheduleCommand::from_byte(bytes[0]),
            splice_event_id: u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
            time: SpliceScheduleTime::parse(&bytes[5..9])?,
            unique_program_id: u16::from_be_bytes([bytes[9], bytes[10]]),
            auto_return: bytes[11],
            break_duration: u16::from_be_bytes([bytes[12], bytes[13]]),
            avail_num: bytes[14],
            avails_expected: bytes[15],
        })
    }
}

impl Serialize for ScheduleDefinition {
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
        buf[0] = self.splice_schedule_command.to_byte();
        buf[1..5].copy_from_slice(&self.splice_event_id.to_be_bytes());
        self.time.serialize_into(&mut buf[5..9])?;
        buf[9..11].copy_from_slice(&self.unique_program_id.to_be_bytes());
        buf[11] = self.auto_return;
        buf[12..14].copy_from_slice(&self.break_duration.to_be_bytes());
        buf[14] = self.avail_num;
        buf[15] = self.avails_expected;
        Ok(LEN)
    }
}

impl OperationDef<'_> for ScheduleDefinition {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "SCHEDULE_DEFINITION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = ScheduleDefinition {
            splice_schedule_command: SpliceScheduleCommand::SpliceInsert,
            splice_event_id: 0x42,
            time: SpliceScheduleTime {
                seconds: 0x6000_0000,
            },
            unique_program_id: 1,
            auto_return: 1,
            break_duration: 300,
            avail_num: 0,
            avails_expected: 0,
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), LEN);
        let back = ScheduleDefinition::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = ScheduleDefinition {
            splice_event_id: 1,
            ..ScheduleDefinition::default()
        };
        let bytes = op.to_bytes();
        let mut op2 = op;
        op2.splice_event_id = 999;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
