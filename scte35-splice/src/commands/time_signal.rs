//! time_signal() — ANSI/SCTE 35 2023r1 §9.7.4, Table 11
//! (splice_command_type 0x06).
//!
//! A single `splice_time()` structure; its payload (segmentation, etc.) rides
//! in the section descriptor loop. When `time_specified_flag` is 0 the command
//! is interpreted as immediate.

use crate::error::{Error, Result};
use crate::time::SpliceTime;
use crate::traits::CommandDef;
use dvb_common::{Parse, Serialize};

/// `splice_command_type` for time_signal (§9.6.1, Table 7).
pub const COMMAND_TYPE: u8 = 0x06;

/// time_signal() — §9.7.4, Table 11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TimeSignal {
    /// The splice time (§9.8.1).
    pub splice_time: SpliceTime,
}

impl<'a> Parse<'a> for TimeSignal {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            splice_time: SpliceTime::parse(bytes)?,
        })
    }
}

impl Serialize for TimeSignal {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.splice_time.serialized_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.splice_time.serialize_into(buf)
    }
}

impl<'a> CommandDef<'a> for TimeSignal {
    const COMMAND_TYPE: u8 = COMMAND_TYPE;
    const NAME: &'static str = "TIME_SIGNAL";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_time() {
        let cmd = TimeSignal {
            splice_time: SpliceTime::with_pts(0x0_1234_5678),
        };
        let bytes = cmd.to_bytes();
        let back = TimeSignal::parse(&bytes).unwrap();
        assert_eq!(cmd, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn round_trip_immediate() {
        let cmd = TimeSignal::default();
        let bytes = cmd.to_bytes();
        assert_eq!(bytes.len(), 1);
        assert_eq!(TimeSignal::parse(&bytes).unwrap(), cmd);
    }
}
