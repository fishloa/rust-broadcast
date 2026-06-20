//! transmit_schedule_request_data() — ANSI/SCTE 104 2023 §9.7.4, Table 9-22 (opID 0x0105).
//!
//! Normal usage request. Transmits accumulated schedule data. A `cancel`
//! flag aborts a pending schedule.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use dvb_common::{Parse, Serialize};

/// `opID` for transmit_schedule_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0105;

/// Fixed wire length.
pub const LEN: usize = 1;

/// transmit_schedule_request_data() — §9.7.4, Table 9-22.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TransmitSchedule {
    /// `cancel` — 1 byte. Non-zero = cancel download.
    pub cancel: u8,
}

impl<'a> Parse<'a> for TransmitSchedule {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "transmit_schedule cancel",
            });
        }
        Ok(Self { cancel: bytes[0] })
    }
}

impl Serialize for TransmitSchedule {
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
        buf[0] = self.cancel;
        Ok(LEN)
    }
}

impl<'a> OperationDef<'a> for TransmitSchedule {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "TRANSMIT_SCHEDULE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = TransmitSchedule { cancel: 0 };
        let bytes = op.to_bytes();
        assert_eq!(bytes, [0]);
        let back = TransmitSchedule::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn cancel_round_trip() {
        let op = TransmitSchedule { cancel: 1 };
        let bytes = op.to_bytes();
        assert_eq!(bytes, [1]);
        let back = TransmitSchedule::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = TransmitSchedule { cancel: 0 };
        let bytes = op.to_bytes();
        let mut op2 = op;
        op2.cancel = 1;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
