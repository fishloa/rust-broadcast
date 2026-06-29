//! insert_DTMF_descriptor_request_data() — ANSI/SCTE 104 2023 §9.8.6, Table 9-28 (opID 0x0109).
//!
//! Supplemental usage. Creates a DTMF descriptor in the resulting
//! SCTE 35 section. Carries a variable-length loop of DTMF characters.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for insert_DTMF_descriptor_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0109;

/// insert_DTMF_descriptor_request_data() — §9.8.6, Table 9-28.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertDtmfDescriptor<'a> {
    /// `pre_roll` — 1 byte, tenths of seconds.
    pub pre_roll: u8,
    /// DTMF characters (raw bytes).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub dtmf_chars: &'a [u8],
}

impl<'a> Parse<'a> for InsertDtmfDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: bytes.len(),
                what: "insert_DTMF_descriptor pre_roll+dtmf_length",
            });
        }
        let pre_roll = bytes[0];
        let dtmf_length = bytes[1] as usize;
        if bytes.len() < 2 + dtmf_length {
            return Err(Error::BufferTooShort {
                need: 2 + dtmf_length,
                have: bytes.len(),
                what: "insert_DTMF_descriptor DTMF chars",
            });
        }
        Ok(Self {
            pre_roll,
            dtmf_chars: &bytes[2..2 + dtmf_length],
        })
    }
}

impl Serialize for InsertDtmfDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        2 + self.dtmf_chars.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.pre_roll;
        buf[1] = self.dtmf_chars.len() as u8;
        buf[2..2 + self.dtmf_chars.len()].copy_from_slice(self.dtmf_chars);
        Ok(need)
    }
}

impl<'a> OperationDef<'a> for InsertDtmfDescriptor<'a> {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_DTMF_DESCRIPTOR";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let chars = &[0x31, 0x32, 0x33][..]; // '1','2','3'
        let op = InsertDtmfDescriptor {
            pre_roll: 30,
            dtmf_chars: chars,
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), 5);
        let back = InsertDtmfDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn empty_dtmf_round_trip() {
        let op = InsertDtmfDescriptor {
            pre_roll: 0,
            dtmf_chars: &[],
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), 2);
        let back = InsertDtmfDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InsertDtmfDescriptor {
            pre_roll: 30,
            dtmf_chars: &[0x31],
        };
        let bytes = op.to_bytes();
        let mut op2 = op;
        op2.pre_roll = 60;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
