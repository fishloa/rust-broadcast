//! splice_insert() — ANSI/SCTE 35 2023r1 §9.7.3, Table 10
//! (splice_command_type 0x05).
//!
//! Signals an upcoming splice event. Its splice mode is one enum rather than a
//! pair of flags beside parallel options: [`SpliceMode::Program`] carries
//! either [`ProgramSplice::Immediate`] (no `splice_time()`) or
//! [`ProgramSplice::At`] (one `splice_time()`), and [`SpliceMode::Components`]
//! is the deprecated Component Splice Mode, parsed and serialized losslessly
//! (`splice_immediate_flag` then decides whether every component carries its
//! own `splice_time()`). The wire flags are derived from the value, so a
//! populated field can never contradict a flag and be dropped on serialize.

use alloc::vec::Vec;
use broadcast_common::len::{need_in, need_out};

use crate::error::{Error, Result};
use crate::time::{BreakDuration, SpliceTime};
use crate::traits::CommandDef;
use broadcast_common::{Parse, Serialize};

/// `splice_command_type` for splice_insert (§9.6.1, Table 7).
pub const COMMAND_TYPE: u8 = 0x05;

/// Program Splice Mode (`program_splice_flag == 1`): the whole program splices
/// at one point, chosen now (immediate) or at a stated `pts_time`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ProgramSplice {
    /// `splice_immediate_flag == 1`: no `splice_time()`; the splicer picks the
    /// nearest opportunity. A `pts_time` cannot ride with this value.
    Immediate,
    /// `splice_immediate_flag == 0`: the program splices at this
    /// `splice_time()`.
    At(SpliceTime),
}

/// How a [`SpliceInsert`] splices: the `program_splice_flag` /
/// `splice_immediate_flag` / `component_count` block of §9.7.3, Table 10.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SpliceMode {
    /// `program_splice_flag == 1`: one splice point for the whole program.
    Program(ProgramSplice),
    /// `program_splice_flag == 0` (deprecated Component Splice Mode): the
    /// component loop. `immediate == false` (`splice_immediate_flag == 0`)
    /// gives every component its own `splice_time()`;
    /// `immediate == true` gives none of them one.
    Components {
        /// `splice_immediate_flag` for the component loop.
        immediate: bool,
        /// The `component_count` loop entries, in wire order.
        components: Vec<SpliceInsertComponent>,
    },
}

impl Default for SpliceMode {
    fn default() -> Self {
        Self::Program(ProgramSplice::At(SpliceTime::default()))
    }
}

impl SpliceMode {
    /// `program_splice_flag` as carried on the wire.
    #[must_use]
    pub const fn program_splice_flag(&self) -> bool {
        matches!(self, Self::Program(_))
    }

    /// `splice_immediate_flag` as carried on the wire.
    #[must_use]
    pub const fn splice_immediate_flag(&self) -> bool {
        match self {
            Self::Program(program) => matches!(program, ProgramSplice::Immediate),
            Self::Components { immediate, .. } => *immediate,
        }
    }

    fn serialized_len(&self) -> usize {
        match self {
            Self::Program(ProgramSplice::Immediate) => 0,
            Self::Program(ProgramSplice::At(splice_time)) => splice_time.serialized_len(),
            Self::Components {
                immediate,
                components,
            } => {
                let mut len = 1; // component_count
                for c in components {
                    len += 1; // component_tag
                    if !immediate {
                        len += c.splice_time.unwrap_or_default().serialized_len();
                    }
                }
                len
            }
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        match self {
            Self::Program(ProgramSplice::Immediate) => Ok(0),
            Self::Program(ProgramSplice::At(splice_time)) => splice_time.serialize_into(buf),
            Self::Components {
                immediate,
                components,
            } => {
                buf[0] = broadcast_common::len::fit_u8(
                    components.len(),
                    "splice_insert.component_count",
                )?;
                let mut pos = 1;
                for c in components {
                    buf[pos] = c.component_tag;
                    pos += 1;
                    if !immediate {
                        pos += c
                            .splice_time
                            .unwrap_or_default()
                            .serialize_into(&mut buf[pos..])?;
                    }
                }
                Ok(need)
            }
        }
    }
}

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

impl SpliceInsertComponent {
    /// One component entry with the given tag and (optional) splice time.
    #[must_use]
    pub const fn new(component_tag: u8, splice_time: Option<SpliceTime>) -> Self {
        Self {
            component_tag,
            splice_time,
        }
    }
}

/// splice_insert() — §9.7.3, Table 10.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceInsert {
    /// 32-bit unique `splice_event_id` (§9.9.3).
    pub splice_event_id: u32,
    /// When `Some(id)`, the event `id` has been cancelled
    /// (`splice_event_cancel_indicator == 1`) and no further fields are
    /// present. Build it with [`SpliceInsert::cancel`], which keeps the two ids
    /// in step with the single wire event id.
    pub cancel: Option<u32>,
    /// `out_of_network_indicator` (present only when not cancelled).
    pub out_of_network_indicator: bool,
    /// `event_id_compliance_flag`: `false` = event id complies with §9.9.3.
    pub event_id_compliance_flag: bool,
    /// The splice mode; its variant fixes `program_splice_flag`,
    /// `splice_immediate_flag` and the component loop.
    pub mode: SpliceMode,
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
            cancel: None,
            out_of_network_indicator: false,
            event_id_compliance_flag: true,
            mode: SpliceMode::default(),
            break_duration: None,
            unique_program_id: 0,
            avail_num: 0,
            avails_expected: 0,
        }
    }
}

impl SpliceInsert {
    /// The command that cancels `splice_event_id`: `splice_event_cancel_indicator
    /// == 1`, after which no other field is present.
    #[must_use]
    pub fn cancel(splice_event_id: u32) -> Self {
        Self {
            splice_event_id,
            cancel: Some(splice_event_id),
            ..Self::default()
        }
    }
}

impl<'a> Parse<'a> for SpliceInsert {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // splice_event_id (4) + flags byte (1).
        let (hdr, _) = bytes.split_first_chunk::<5>().ok_or(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 5,
                have: bytes.len(),
                what: "splice_insert header",
            },
        ))?;
        let splice_event_id = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let cancel = hdr[4] & 0x80 != 0;
        let mut pos = 5;

        let mut out = Self {
            splice_event_id,
            cancel: cancel.then_some(splice_event_id),
            ..Self::default()
        };

        if cancel {
            return Ok(out);
        }

        need_in(bytes, pos + 1, "splice_insert flags")?;
        let flags = bytes[pos];
        pos += 1;
        out.out_of_network_indicator = flags & 0x80 != 0;
        let program_splice_flag = flags & 0x40 != 0;
        let duration_flag = flags & 0x20 != 0;
        let splice_immediate_flag = flags & 0x10 != 0;
        out.event_id_compliance_flag = flags & 0x08 != 0;

        out.mode = if program_splice_flag {
            if splice_immediate_flag {
                SpliceMode::Program(ProgramSplice::Immediate)
            } else {
                let splice_time = SpliceTime::parse(&bytes[pos..])?;
                pos += splice_time.serialized_len();
                SpliceMode::Program(ProgramSplice::At(splice_time))
            }
        } else {
            // Component Splice Mode (deprecated).
            need_in(bytes, pos + 1, "splice_insert component_count")?;
            let component_count = bytes[pos] as usize;
            pos += 1;
            let mut components = Vec::with_capacity(component_count);
            for _ in 0..component_count {
                need_in(bytes, pos + 1, "splice_insert component_tag")?;
                let component_tag = bytes[pos];
                pos += 1;
                let splice_time = if splice_immediate_flag {
                    None
                } else {
                    let splice_time = SpliceTime::parse(&bytes[pos..])?;
                    pos += splice_time.serialized_len();
                    Some(splice_time)
                };
                components.push(SpliceInsertComponent {
                    component_tag,
                    splice_time,
                });
            }
            SpliceMode::Components {
                immediate: splice_immediate_flag,
                components,
            }
        };

        if duration_flag {
            let bd = BreakDuration::parse(&bytes[pos..])?;
            pos += BreakDuration::LEN;
            out.break_duration = Some(bd);
        }

        let (trailer, _) = bytes[pos..]
            .split_first_chunk::<4>()
            .ok_or(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: pos + 4,
                    have: bytes.len(),
                    what: "splice_insert trailer",
                },
            ))?;
        out.unique_program_id = u16::from_be_bytes([trailer[0], trailer[1]]);
        out.avail_num = trailer[2];
        out.avails_expected = trailer[3];
        Ok(out)
    }
}

impl Serialize for SpliceInsert {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.cancel.is_some() {
            return 5;
        }
        let mut len = 6; // event_id (4) + cancel/reserved (1) + flags (1)
        len += self.mode.serialized_len();
        if self.break_duration.is_some() {
            len += BreakDuration::LEN;
        }
        len += 4; // unique_program_id (2) + avail_num (1) + avails_expected (1)
        len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[0..4].copy_from_slice(&self.splice_event_id.to_be_bytes());
        // cancel (1) + 7 reserved bits = 1.
        buf[4] = (u8::from(self.cancel.is_some()) << 7) | 0x7F;
        if let Some(cancelled) = self.cancel {
            if cancelled != self.splice_event_id {
                return Err(Error::InvalidField(broadcast_common::error::InvalidField {
                    field: "splice_insert.splice_event_cancel_indicator",
                    reason: "a cancel names one event, so splice_event_id and the cancelled id must agree",
                }));
            }
            return Ok(5);
        }

        // flags byte: out_of_network, program_splice, duration, immediate,
        // compliance, then 3 reserved bits = 1.
        let duration_flag = self.break_duration.is_some();
        buf[5] = (u8::from(self.out_of_network_indicator) << 7)
            | (u8::from(self.mode.program_splice_flag()) << 6)
            | (u8::from(duration_flag) << 5)
            | (u8::from(self.mode.splice_immediate_flag()) << 4)
            | (u8::from(self.event_id_compliance_flag) << 3)
            | 0x07;
        let mut pos = 6;
        pos += self.mode.serialize_into(&mut buf[pos..])?;

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
    use alloc::vec;
    use broadcast_common::clock33::Clock33;

    fn rt(cmd: &SpliceInsert) {
        let bytes = cmd.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), cmd.serialized_len());
        let back = SpliceInsert::parse(&bytes).unwrap();
        assert_eq!(*cmd, back);
        assert_eq!(back.try_to_bytes().unwrap(), bytes);
    }

    fn at(pts: u64) -> SpliceTime {
        SpliceTime::at(Clock33::try_new(pts).expect("in range"))
    }

    #[test]
    fn round_trip_cancel() {
        rt(&SpliceInsert::cancel(0xDEAD_BEEF));
    }

    #[test]
    fn round_trip_program_with_time_and_duration() {
        rt(&SpliceInsert {
            splice_event_id: 0x4800_0001,
            out_of_network_indicator: true,
            event_id_compliance_flag: false,
            mode: SpliceMode::Program(ProgramSplice::At(at(0x0_0123_4567))),
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

    /// SC-W3 (#1129): `component_count` is an 8-bit field but used to be
    /// written with `self.components.len() as u8`, which wraps for 256+
    /// components while all of them are still serialized.
    #[test]
    fn over_255_components_rejected_not_wrapped() {
        let cmd = SpliceInsert {
            splice_event_id: 1,
            mode: SpliceMode::Components {
                immediate: true,
                components: (0..256)
                    .map(|i| SpliceInsertComponent::new(i as u8, None))
                    .collect(),
            },
            ..Default::default()
        };
        assert!(matches!(
            &(cmd.try_to_bytes()),
            Err(crate::error::Error::FieldOverflow(_))
        ));
    }

    #[test]
    fn round_trip_program_immediate() {
        rt(&SpliceInsert {
            splice_event_id: 0x6000_0009,
            mode: SpliceMode::Program(ProgramSplice::Immediate),
            unique_program_id: 7,
            ..Default::default()
        });
    }

    #[test]
    fn round_trip_component_mode() {
        rt(&SpliceInsert {
            splice_event_id: 0xC000_0002,
            mode: SpliceMode::Components {
                immediate: false,
                components: vec![
                    SpliceInsertComponent::new(1, Some(at(0x1000))),
                    SpliceInsertComponent::new(2, Some(SpliceTime::default())),
                ],
            },
            unique_program_id: 9,
            ..Default::default()
        });
    }

    /// Audit 19b, finding 14: the old `SpliceInsert` had
    /// `program_splice_flag` / `splice_immediate_flag` beside
    /// `splice_time: Option<SpliceTime>` / `components`, and the serializer
    /// followed the flags. A scheduled cue (`splice_time: Some(t)`,
    /// `splice_immediate_flag: true`) serialized as an immediate cue with `t`
    /// dropped, and re-parsing returned a different command (confirmed on the
    /// old wire: the emitted section lost the 5 `splice_time()` bytes). The
    /// mode is one value now, so that struct no longer compiles and its wire
    /// form is unrepresentable: an immediate program splice has no
    /// `splice_time()` field to hold a time.
    #[test]
    fn immediate_program_splice_carries_no_time() {
        let immediate = SpliceInsert {
            splice_event_id: 7,
            mode: SpliceMode::Program(ProgramSplice::Immediate),
            ..Default::default()
        };
        let bytes = immediate.try_to_bytes().unwrap();
        assert_eq!(
            bytes[5],
            0x40 | 0x10 | 0x08 | 0x07,
            "program_splice, immediate, compliance, reserved"
        );
        assert_eq!(SpliceInsert::parse(&bytes).unwrap(), immediate);
        // The same command with a time is five bytes longer: the time is
        // carried, not silently discarded by a flag beside it.
        let mut scheduled = immediate.clone();
        scheduled.mode = SpliceMode::Program(ProgramSplice::At(at(90_000)));
        assert_eq!(scheduled.serialized_len(), immediate.serialized_len() + 5);
        rt(&scheduled);
    }

    /// A cancel byte and `splice_event_id` cannot disagree on the wire, so a
    /// cancel that names a different id is rejected instead of being written
    /// as a cancel of `splice_event_id`.
    #[test]
    fn cancel_naming_another_event_is_rejected() {
        let cmd = SpliceInsert {
            splice_event_id: 1,
            cancel: Some(2),
            ..Default::default()
        };
        assert!(matches!(
            cmd.try_to_bytes(),
            Err(crate::error::Error::InvalidField(
                broadcast_common::error::InvalidField {
                    field: "splice_insert.splice_event_cancel_indicator",
                    ..
                }
            ))
        ));
    }
}
