//! insert_alternate_break_duration request — ANSI/SCTE 104 2023 §9.8.13,
//! Table 9-36 (opID 0x0113).
//!
//! Supplemental usage. A single 32-bit `alternate_break_duration` in
//! **milliseconds** that replaces the `break_duration` of the associated SCTE 35
//! section (zero means "use the `break_duration()`"). Transcription:
//! `docs/ansi_scte_104/audio-and-alternate-break.md`.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for insert_alternate_break_duration request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0113;

/// Wire size of the body.
pub const LEN: usize = 4;

/// insert_alternate_break_duration request — §9.8.13, Table 9-36.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertAlternateBreakDuration {
    /// `alternate_break_duration` — 32 bits, milliseconds.
    pub alternate_break_duration: u32,
}

impl<'a> Parse<'a> for InsertAlternateBreakDuration {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (head, _) = bytes
            .split_first_chunk::<LEN>()
            .ok_or(Error::BufferTooShort {
                need: LEN,
                have: bytes.len(),
                what: "insert_alternate_break_duration",
            })?;
        Ok(Self {
            alternate_break_duration: u32::from_be_bytes(*head),
        })
    }
}

impl Serialize for InsertAlternateBreakDuration {
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
        buf[..LEN].copy_from_slice(&self.alternate_break_duration.to_be_bytes());
        Ok(LEN)
    }
}

impl OperationDef<'_> for InsertAlternateBreakDuration {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_ALTERNATE_BREAK_DURATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_layout_is_a_big_endian_u32_of_milliseconds() {
        let op = InsertAlternateBreakDuration {
            alternate_break_duration: 30_000,
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes, [0x00, 0x00, 0x75, 0x30]);
        assert_eq!(InsertAlternateBreakDuration::parse(&bytes).unwrap(), op);
    }

    #[test]
    fn short_body_is_rejected() {
        assert!(InsertAlternateBreakDuration::parse(&[0, 0, 0]).is_err());
    }
}
