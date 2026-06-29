//! splice_schedule() — ANSI/SCTE 35 2023r1 §9.7.2, Table 9
//! (splice_command_type 0x04).
//!
//! A schedule of future splice events, each timed by a 32-bit `utc_splice_time`
//! (GPS-epoch seconds, §9.7.2.1). Component Splice Mode
//! (`program_splice_flag == 0`) is deprecated but parsed/serialized losslessly.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::time::BreakDuration;
use crate::traits::CommandDef;
use broadcast_common::{Parse, Serialize};

/// `splice_command_type` for splice_schedule (§9.6.1, Table 7).
pub const COMMAND_TYPE: u8 = 0x04;

/// One component entry in a scheduled event's deprecated component loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceScheduleComponent {
    /// 8-bit `component_tag`.
    pub component_tag: u8,
    /// 32-bit `utc_splice_time` (GPS-epoch seconds).
    pub utc_splice_time: u32,
}

/// One scheduled splice event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceScheduleEvent {
    /// 32-bit unique `splice_event_id` (§9.9.3).
    pub splice_event_id: u32,
    /// When `true`, the named event has been cancelled.
    pub splice_event_cancel_indicator: bool,
    /// `event_id_compliance_flag`: `false` = event id complies with §9.9.3.
    pub event_id_compliance_flag: bool,
    /// `out_of_network_indicator` (present only when not cancelled).
    pub out_of_network_indicator: bool,
    /// `program_splice_flag`: `true` = Program Splice Mode.
    pub program_splice_flag: bool,
    /// Program-mode `utc_splice_time`, present when `program_splice_flag == 1`.
    pub utc_splice_time: Option<u32>,
    /// Component-mode entries, present when `program_splice_flag == 0`
    /// (deprecated).
    pub components: Vec<SpliceScheduleComponent>,
    /// `break_duration()`, present when `duration_flag == 1`.
    pub break_duration: Option<BreakDuration>,
    /// `unique_program_id`.
    pub unique_program_id: u16,
    /// `avail_num`.
    pub avail_num: u8,
    /// `avails_expected`.
    pub avails_expected: u8,
}

impl Default for SpliceScheduleEvent {
    fn default() -> Self {
        Self {
            splice_event_id: 0,
            splice_event_cancel_indicator: false,
            event_id_compliance_flag: true,
            out_of_network_indicator: false,
            program_splice_flag: true,
            utc_splice_time: None,
            components: Vec::new(),
            break_duration: None,
            unique_program_id: 0,
            avail_num: 0,
            avails_expected: 0,
        }
    }
}

impl SpliceScheduleEvent {
    fn serialized_len(&self) -> usize {
        // splice_event_id (4) + flags byte (1).
        let mut len = 5;
        if self.splice_event_cancel_indicator {
            return len;
        }
        len += 1; // out_of_network/program/duration/reserved byte
        if self.program_splice_flag {
            len += 4; // utc_splice_time
        } else {
            len += 1; // component_count
            len += self.components.len() * 5; // tag (1) + utc_splice_time (4)
        }
        if self.break_duration.is_some() {
            len += BreakDuration::LEN;
        }
        len += 4; // unique_program_id (2) + avail_num (1) + avails_expected (1)
        len
    }

    fn parse_at(bytes: &[u8], pos: &mut usize) -> Result<Self> {
        let (hdr, _) = bytes[*pos..]
            .split_first_chunk::<5>()
            .ok_or(Error::BufferTooShort {
                need: *pos + 5,
                have: bytes.len(),
                what: "splice_schedule event header",
            })?;
        let splice_event_id = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let b = hdr[4];
        *pos += 5;
        let cancel = b & 0x80 != 0;
        let compliance = b & 0x40 != 0;

        let mut ev = Self {
            splice_event_id,
            splice_event_cancel_indicator: cancel,
            event_id_compliance_flag: compliance,
            ..Self::default()
        };
        if cancel {
            return Ok(ev);
        }

        if bytes.len() < *pos + 1 {
            return Err(Error::BufferTooShort {
                need: *pos + 1,
                have: bytes.len(),
                what: "splice_schedule event flags",
            });
        }
        let flags = bytes[*pos];
        *pos += 1;
        ev.out_of_network_indicator = flags & 0x80 != 0;
        ev.program_splice_flag = flags & 0x40 != 0;
        let duration_flag = flags & 0x20 != 0;

        if ev.program_splice_flag {
            let (t, _) = bytes[*pos..]
                .split_first_chunk::<4>()
                .ok_or(Error::BufferTooShort {
                    need: *pos + 4,
                    have: bytes.len(),
                    what: "splice_schedule utc_splice_time",
                })?;
            ev.utc_splice_time = Some(u32::from_be_bytes(*t));
            *pos += 4;
        } else {
            if bytes.len() < *pos + 1 {
                return Err(Error::BufferTooShort {
                    need: *pos + 1,
                    have: bytes.len(),
                    what: "splice_schedule component_count",
                });
            }
            let count = bytes[*pos] as usize;
            *pos += 1;
            for _ in 0..count {
                let (comp, _) =
                    bytes[*pos..]
                        .split_first_chunk::<5>()
                        .ok_or(Error::BufferTooShort {
                            need: *pos + 5,
                            have: bytes.len(),
                            what: "splice_schedule component",
                        })?;
                let component_tag = comp[0];
                let utc_splice_time = u32::from_be_bytes([comp[1], comp[2], comp[3], comp[4]]);
                *pos += 5;
                ev.components.push(SpliceScheduleComponent {
                    component_tag,
                    utc_splice_time,
                });
            }
        }

        if duration_flag {
            ev.break_duration = Some(BreakDuration::parse(&bytes[*pos..])?);
            *pos += BreakDuration::LEN;
        }

        let (trailer, _) = bytes[*pos..]
            .split_first_chunk::<4>()
            .ok_or(Error::BufferTooShort {
                need: *pos + 4,
                have: bytes.len(),
                what: "splice_schedule event trailer",
            })?;
        ev.unique_program_id = u16::from_be_bytes([trailer[0], trailer[1]]);
        ev.avail_num = trailer[2];
        ev.avails_expected = trailer[3];
        *pos += 4;
        Ok(ev)
    }

    fn serialize_at(&self, buf: &mut [u8], pos: &mut usize) -> Result<()> {
        buf[*pos..*pos + 4].copy_from_slice(&self.splice_event_id.to_be_bytes());
        // cancel (1) + compliance (1) + 6 reserved bits = 1.
        buf[*pos + 4] = (u8::from(self.splice_event_cancel_indicator) << 7)
            | (u8::from(self.event_id_compliance_flag) << 6)
            | 0x3F;
        *pos += 5;
        if self.splice_event_cancel_indicator {
            return Ok(());
        }

        let duration_flag = self.break_duration.is_some();
        // out_of_network (1) + program_splice (1) + duration (1) + 5 reserved.
        buf[*pos] = (u8::from(self.out_of_network_indicator) << 7)
            | (u8::from(self.program_splice_flag) << 6)
            | (u8::from(duration_flag) << 5)
            | 0x1F;
        *pos += 1;

        if self.program_splice_flag {
            buf[*pos..*pos + 4].copy_from_slice(&self.utc_splice_time.unwrap_or(0).to_be_bytes());
            *pos += 4;
        } else {
            buf[*pos] = self.components.len() as u8;
            *pos += 1;
            for c in &self.components {
                buf[*pos] = c.component_tag;
                buf[*pos + 1..*pos + 5].copy_from_slice(&c.utc_splice_time.to_be_bytes());
                *pos += 5;
            }
        }

        if let Some(bd) = self.break_duration {
            *pos += bd.serialize_into(&mut buf[*pos..])?;
        }

        buf[*pos..*pos + 2].copy_from_slice(&self.unique_program_id.to_be_bytes());
        buf[*pos + 2] = self.avail_num;
        buf[*pos + 3] = self.avails_expected;
        *pos += 4;
        Ok(())
    }
}

/// splice_schedule() — §9.7.2, Table 9.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceSchedule {
    /// The scheduled events (the `splice_count` loop).
    pub events: Vec<SpliceScheduleEvent>,
}

impl<'a> Parse<'a> for SpliceSchedule {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "splice_schedule splice_count",
            });
        }
        let count = bytes[0] as usize;
        let mut pos = 1;
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            events.push(SpliceScheduleEvent::parse_at(bytes, &mut pos)?);
        }
        Ok(Self { events })
    }
}

impl Serialize for SpliceSchedule {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        1 + self
            .events
            .iter()
            .map(SpliceScheduleEvent::serialized_len)
            .sum::<usize>()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.events.len() as u8;
        let mut pos = 1;
        for ev in &self.events {
            ev.serialize_at(buf, &mut pos)?;
        }
        Ok(need)
    }
}

impl<'a> CommandDef<'a> for SpliceSchedule {
    const COMMAND_TYPE: u8 = COMMAND_TYPE;
    const NAME: &'static str = "SPLICE_SCHEDULE";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(cmd: &SpliceSchedule) {
        let bytes = cmd.to_bytes();
        assert_eq!(bytes.len(), cmd.serialized_len());
        let back = SpliceSchedule::parse(&bytes).unwrap();
        assert_eq!(*cmd, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn round_trip_empty_schedule() {
        rt(&SpliceSchedule::default());
    }

    #[test]
    fn round_trip_program_events() {
        rt(&SpliceSchedule {
            events: vec![
                SpliceScheduleEvent {
                    splice_event_id: 1,
                    out_of_network_indicator: true,
                    utc_splice_time: Some(0x1234_5678),
                    break_duration: Some(BreakDuration {
                        auto_return: false,
                        duration: 100,
                    }),
                    unique_program_id: 5,
                    avail_num: 1,
                    avails_expected: 2,
                    ..Default::default()
                },
                SpliceScheduleEvent {
                    splice_event_id: 2,
                    splice_event_cancel_indicator: true,
                    ..Default::default()
                },
            ],
        });
    }

    #[test]
    fn round_trip_component_event() {
        rt(&SpliceSchedule {
            events: vec![SpliceScheduleEvent {
                splice_event_id: 3,
                program_splice_flag: false,
                components: vec![
                    SpliceScheduleComponent {
                        component_tag: 1,
                        utc_splice_time: 0xAABBCCDD,
                    },
                    SpliceScheduleComponent {
                        component_tag: 2,
                        utc_splice_time: 0x11223344,
                    },
                ],
                unique_program_id: 9,
                ..Default::default()
            }],
        });
    }
}
