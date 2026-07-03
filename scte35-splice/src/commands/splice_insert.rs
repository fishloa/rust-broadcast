//! splice_insert() — ANSI/SCTE 35 2023r1 §9.7.3, Table 10
//! (splice_command_type 0x05).
//!
//! Signals an upcoming splice event. Program Splice Mode
//! (`program_splice_flag == 1`) is the supported mode; Component Splice Mode
//! (flag 0) is deprecated but parsed/serialized losslessly via
//! [`SpliceInsert::components`].

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::time::{BreakDuration, SpliceTime};
use crate::traits::CommandDef;
use broadcast_common::{Parse, Serialize};

/// `splice_command_type` for splice_insert (§9.6.1, Table 7).
pub const COMMAND_TYPE: u8 = 0x05;

/// One component entry in the deprecated Component Splice Mode loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceInsertComponent {
    /// 8-bit `component_tag` (matches the PMT stream_identifier_descriptor()).
    pub component_tag: u8,
    /// Per-component `splice_time()`, present only when
    /// `splice_immediate_flag == 0`.
    pub splice_time: Option<SpliceTime>,
}

/// splice_insert() — §9.7.3, Table 10.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceInsert {
    /// 32-bit unique `splice_event_id` (§9.9.3).
    pub splice_event_id: u32,
    /// When `true`, the named event has been cancelled and no further fields
    /// are present.
    pub splice_event_cancel_indicator: bool,
    /// `out_of_network_indicator` (present only when not cancelled).
    pub out_of_network_indicator: bool,
    /// `program_splice_flag`: `true` = Program Splice Mode (supported);
    /// `false` = Component Splice Mode (deprecated).
    pub program_splice_flag: bool,
    /// `splice_immediate_flag`: when `true`, no `splice_time()` is present and
    /// the splicer chooses the nearest opportunity.
    pub splice_immediate_flag: bool,
    /// `event_id_compliance_flag`: `false` = event id complies with §9.9.3.
    pub event_id_compliance_flag: bool,
    /// Program-mode `splice_time()`, present only when
    /// `program_splice_flag == 1 && splice_immediate_flag == 0`.
    pub splice_time: Option<SpliceTime>,
    /// Component-mode entries, present only when `program_splice_flag == 0`
    /// (deprecated).
    pub components: Vec<SpliceInsertComponent>,
    /// `break_duration()`, present only when `duration_flag == 1`.
    pub break_duration: Option<BreakDuration>,
    /// `unique_program_id`.
    pub unique_program_id: u16,
    /// `avail_num`.
    pub avail_num: u8,
    /// `avails_expected`.
    pub avails_expected: u8,
}

impl Default for SpliceInsert {
    fn default() -> Self {
        Self {
            splice_event_id: 0,
            splice_event_cancel_indicator: false,
            out_of_network_indicator: false,
            program_splice_flag: true,
            splice_immediate_flag: false,
            event_id_compliance_flag: true,
            splice_time: None,
            components: Vec::new(),
            break_duration: None,
            unique_program_id: 0,
            avail_num: 0,
            avails_expected: 0,
        }
    }
}

impl<'a> Parse<'a> for SpliceInsert {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // splice_event_id (4) + flags byte (1).
        let (hdr, _) = bytes
            .split_first_chunk::<5>()
            .ok_or(Error::BufferTooShort {
                need: 5,
                have: bytes.len(),
                what: "splice_insert header",
            })?;
        let splice_event_id = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let cancel = hdr[4] & 0x80 != 0;
        let mut pos = 5;

        let mut out = Self {
            splice_event_id,
            splice_event_cancel_indicator: cancel,
            program_splice_flag: true,
            event_id_compliance_flag: true,
            ..Self::default()
        };

        if cancel {
            return Ok(out);
        }

        if bytes.len() < pos + 1 {
            return Err(Error::BufferTooShort {
                need: pos + 1,
                have: bytes.len(),
                what: "splice_insert flags",
            });
        }
        let flags = bytes[pos];
        pos += 1;
        out.out_of_network_indicator = flags & 0x80 != 0;
        out.program_splice_flag = flags & 0x40 != 0;
        let duration_flag = flags & 0x20 != 0;
        out.splice_immediate_flag = flags & 0x10 != 0;
        out.event_id_compliance_flag = flags & 0x08 != 0;

        if out.program_splice_flag {
            if !out.splice_immediate_flag {
                let st = SpliceTime::parse(&bytes[pos..])?;
                pos += st.serialized_len();
                out.splice_time = Some(st);
            }
        } else {
            // Component Splice Mode (deprecated).
            if bytes.len() < pos + 1 {
                return Err(Error::BufferTooShort {
                    need: pos + 1,
                    have: bytes.len(),
                    what: "splice_insert component_count",
                });
            }
            let component_count = bytes[pos] as usize;
            pos += 1;
            for _ in 0..component_count {
                if bytes.len() < pos + 1 {
                    return Err(Error::BufferTooShort {
                        need: pos + 1,
                        have: bytes.len(),
                        what: "splice_insert component_tag",
                    });
                }
                let component_tag = bytes[pos];
                pos += 1;
                let splice_time = if !out.splice_immediate_flag {
                    let st = SpliceTime::parse(&bytes[pos..])?;
                    pos += st.serialized_len();
                    Some(st)
                } else {
                    None
                };
                out.components.push(SpliceInsertComponent {
                    component_tag,
                    splice_time,
                });
            }
        }

        if duration_flag {
            let bd = BreakDuration::parse(&bytes[pos..])?;
            pos += BreakDuration::LEN;
            out.break_duration = Some(bd);
        }

        let (trailer, _) = bytes[pos..]
            .split_first_chunk::<4>()
            .ok_or(Error::BufferTooShort {
                need: pos + 4,
                have: bytes.len(),
                what: "splice_insert trailer",
            })?;
        out.unique_program_id = u16::from_be_bytes([trailer[0], trailer[1]]);
        out.avail_num = trailer[2];
        out.avails_expected = trailer[3];
        Ok(out)
    }
}

impl Serialize for SpliceInsert {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.splice_event_cancel_indicator {
            return 5;
        }
        let mut len = 6; // event_id (4) + cancel/reserved (1) + flags (1)
        if self.program_splice_flag {
            if !self.splice_immediate_flag {
                len += self.splice_time.unwrap_or_default().serialized_len();
            }
        } else {
            len += 1; // component_count
            for c in &self.components {
                len += 1; // component_tag
                if !self.splice_immediate_flag {
                    len += c.splice_time.unwrap_or_default().serialized_len();
                }
            }
        }
        if self.break_duration.is_some() {
            len += BreakDuration::LEN;
        }
        len += 4; // unique_program_id (2) + avail_num (1) + avails_expected (1)
        len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.splice_event_id.to_be_bytes());
        // cancel (1) + 7 reserved bits = 1.
        buf[4] = (u8::from(self.splice_event_cancel_indicator) << 7) | 0x7F;
        if self.splice_event_cancel_indicator {
            return Ok(5);
        }

        // flags byte: out_of_network, program_splice, duration, immediate,
        // compliance, then 3 reserved bits = 1.
        let duration_flag = self.break_duration.is_some();
        buf[5] = (u8::from(self.out_of_network_indicator) << 7)
            | (u8::from(self.program_splice_flag) << 6)
            | (u8::from(duration_flag) << 5)
            | (u8::from(self.splice_immediate_flag) << 4)
            | (u8::from(self.event_id_compliance_flag) << 3)
            | 0x07;
        let mut pos = 6;

        if self.program_splice_flag {
            if !self.splice_immediate_flag {
                let st = self.splice_time.unwrap_or_default();
                pos += st.serialize_into(&mut buf[pos..])?;
            }
        } else {
            buf[pos] = self.components.len() as u8;
            pos += 1;
            for c in &self.components {
                buf[pos] = c.component_tag;
                pos += 1;
                if !self.splice_immediate_flag {
                    let st = c.splice_time.unwrap_or_default();
                    pos += st.serialize_into(&mut buf[pos..])?;
                }
            }
        }

        if let Some(bd) = self.break_duration {
            pos += bd.serialize_into(&mut buf[pos..])?;
        }

        buf[pos..pos + 2].copy_from_slice(&self.unique_program_id.to_be_bytes());
        buf[pos + 2] = self.avail_num;
        buf[pos + 3] = self.avails_expected;
        Ok(need)
    }
}

impl CommandDef<'_> for SpliceInsert {
    const COMMAND_TYPE: u8 = COMMAND_TYPE;
    const NAME: &'static str = "SPLICE_INSERT";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(cmd: &SpliceInsert) {
        let bytes = cmd.to_bytes();
        assert_eq!(bytes.len(), cmd.serialized_len());
        let back = SpliceInsert::parse(&bytes).unwrap();
        assert_eq!(*cmd, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn round_trip_cancel() {
        rt(&SpliceInsert {
            splice_event_id: 0xDEADBEEF,
            splice_event_cancel_indicator: true,
            ..Default::default()
        });
    }

    #[test]
    fn round_trip_program_with_time_and_duration() {
        rt(&SpliceInsert {
            splice_event_id: 0x4800_0001,
            out_of_network_indicator: true,
            program_splice_flag: true,
            splice_immediate_flag: false,
            event_id_compliance_flag: false,
            splice_time: Some(SpliceTime::with_pts(0x0_0123_4567)),
            break_duration: Some(BreakDuration {
                auto_return: true,
                duration: 90_000 * 30,
            }),
            unique_program_id: 0x1234,
            avail_num: 1,
            avails_expected: 4,
            ..Default::default()
        });
    }

    #[test]
    fn round_trip_program_immediate() {
        rt(&SpliceInsert {
            splice_event_id: 0x6000_0009,
            program_splice_flag: true,
            splice_immediate_flag: true,
            splice_time: None,
            unique_program_id: 7,
            ..Default::default()
        });
    }

    #[test]
    fn round_trip_component_mode() {
        rt(&SpliceInsert {
            splice_event_id: 0xC000_0002,
            program_splice_flag: false,
            splice_immediate_flag: false,
            components: vec![
                SpliceInsertComponent {
                    component_tag: 1,
                    splice_time: Some(SpliceTime::with_pts(0x1000)),
                },
                SpliceInsertComponent {
                    component_tag: 2,
                    splice_time: Some(SpliceTime::default()),
                },
            ],
            unique_program_id: 9,
            ..Default::default()
        });
    }
}
