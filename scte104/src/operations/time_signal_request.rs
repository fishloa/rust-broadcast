//! time_signal_request_data() — ANSI/SCTE 104 2023 §9.8.1, Table 9-23 (opID 0x0104).
//!
//! Normal usage request. Generates an SCTE 35 time_signal operation at the
//! time indicated by the timestamp. Carries a single `pre_roll_time` field.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for time_signal_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0104;

/// Fixed wire length of `time_signal_request_data()`.
pub const LEN: usize = 2;

/// time_signal_request_data() — §9.8.1, Table 9-23.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TimeSignalRequest {
    /// `pre_roll_time` — 2 bytes, milliseconds.
    pub pre_roll_time: u16,
}

impl<'a> Parse<'a> for TimeSignalRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < LEN {
            return Err(Error::BufferTooShort {
                need: LEN,
                have: bytes.len(),
                what: "time_signal_request_data",
            });
        }
        Ok(Self {
            pre_roll_time: u16::from_be_bytes([bytes[0], bytes[1]]),
        })
    }
}

impl Serialize for TimeSignalRequest {
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
        buf[0..2].copy_from_slice(&self.pre_roll_time.to_be_bytes());
        Ok(LEN)
    }
}

impl OperationDef<'_> for TimeSignalRequest {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "TIME_SIGNAL_REQUEST";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = TimeSignalRequest {
            pre_roll_time: 2000,
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), LEN);
        let back = TimeSignalRequest::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = TimeSignalRequest {
            pre_roll_time: 1000,
        };
        let bytes = op.to_bytes();
        let mut op2 = op;
        op2.pre_roll_time = 2000;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
