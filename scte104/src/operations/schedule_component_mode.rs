//! schedule_component_mode_request_data() — ANSI/SCTE 104 2023 §9.7.3, Table 9-21 (opID 0x010D).
//!
//! Supplemental request for schedule_definition. Specifies per-component
//! splice timing in component mode. Carries a variable-length loop of
//! `(component_tag, time)`.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::time::SpliceScheduleTime;
use crate::traits::OperationDef;
use dvb_common::{Parse, Serialize};

/// `opID` for schedule_component_mode_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x010D;

/// Size of one component entry: 1 (tag) + 4 (time).
pub const ENTRY_LEN: usize = 5;

/// One component entry in `schedule_component_mode_request_data()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ScheduleComponentEntry {
    /// `component_tag` — 1 byte.
    pub component_tag: u8,
    /// `time()` — 4 bytes (seconds since GPS epoch).
    pub time: SpliceScheduleTime,
}

/// schedule_component_mode_request_data() — §9.7.3, Table 9-21.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ScheduleComponentMode {
    /// Loop of component entries.
    pub components: Vec<ScheduleComponentEntry>,
}

impl<'a> Parse<'a> for ScheduleComponentMode {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let count = bytes.len() / ENTRY_LEN;
        if bytes.len() % ENTRY_LEN != 0 {
            return Err(Error::BufferTooShort {
                need: (count + 1) * ENTRY_LEN,
                have: bytes.len(),
                what: "schedule_component_mode data",
            });
        }
        let mut components = Vec::with_capacity(count);
        for i in 0..count {
            let off = i * ENTRY_LEN;
            components.push(ScheduleComponentEntry {
                component_tag: bytes[off],
                time: SpliceScheduleTime::parse(&bytes[off + 1..off + ENTRY_LEN])?,
            });
        }
        Ok(Self { components })
    }
}

impl Serialize for ScheduleComponentMode {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.components.len() * ENTRY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        for (i, c) in self.components.iter().enumerate() {
            let off = i * ENTRY_LEN;
            buf[off] = c.component_tag;
            c.time.serialize_into(&mut buf[off + 1..off + ENTRY_LEN])?;
        }
        Ok(need)
    }
}

impl<'a> OperationDef<'a> for ScheduleComponentMode {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "SCHEDULE_COMPONENT_MODE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = ScheduleComponentMode {
            components: alloc::vec![
                ScheduleComponentEntry {
                    component_tag: 1,
                    time: SpliceScheduleTime {
                        seconds: 0x6000_0000,
                    },
                },
                ScheduleComponentEntry {
                    component_tag: 2,
                    time: SpliceScheduleTime {
                        seconds: 0x6000_0010,
                    },
                },
            ],
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), 10);
        let back = ScheduleComponentMode::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = ScheduleComponentMode {
            components: alloc::vec![ScheduleComponentEntry {
                component_tag: 1,
                time: SpliceScheduleTime { seconds: 100 },
            }],
        };
        let bytes = op.to_bytes();
        let mut op2 = op.clone();
        op2.components[0].time.seconds = 999;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
