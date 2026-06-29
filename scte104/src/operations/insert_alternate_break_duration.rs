//! insert_alternate_break_duration request — ANSI/SCTE 104 2023 §9.8.13 (opID 0x0113).
//!
//! Supplemental usage. Specifies substitution of break duration.
//! The spec transcription for this operation's table was incomplete.
//! The operation body is carried as raw bytes (round-trip safe).

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for insert_alternate_break_duration request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0113;

/// insert_alternate_break_duration request — §9.8.13.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertAlternateBreakDuration<'a> {
    /// Raw operation body (typed struct pending full spec transcription).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub data: &'a [u8],
}

impl<'a> Parse<'a> for InsertAlternateBreakDuration<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self { data: bytes })
    }
}

impl Serialize for InsertAlternateBreakDuration<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[..need].copy_from_slice(self.data);
        Ok(need)
    }
}

impl<'a> OperationDef<'a> for InsertAlternateBreakDuration<'a> {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_ALTERNATE_BREAK_DURATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = InsertAlternateBreakDuration {
            data: &[0x01, 0x02],
        };
        let bytes = op.to_bytes();
        let back = InsertAlternateBreakDuration::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }
}
