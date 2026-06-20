//! proprietary_command_request_data() — ANSI/SCTE 104 2023 §9.8.8, Table 9-30 (opID 0x010C).
//!
//! Normal usage request for vendor-specific extensions. Carries a 32-bit
//! proprietary identifier + 8-bit command + variable-length data.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use dvb_common::{Parse, Serialize};

/// `opID` for proprietary_command_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x010C;

/// Fixed-size portion (proprietary_id + proprietary_command).
pub const FIXED_LEN: usize = 5;

/// proprietary_command_request_data() — §9.8.8, Table 9-30.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProprietaryCommand<'a> {
    /// `proprietary_id` — 4 bytes, registered with SMPTE-RA.
    pub proprietary_id: u32,
    /// `proprietary_command` — 1 byte, opID-like field.
    pub proprietary_command: u8,
    /// Variable-length proprietary data.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub proprietary_data: &'a [u8],
}

impl<'a> Parse<'a> for ProprietaryCommand<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN,
                have: bytes.len(),
                what: "proprietary_command_request_data",
            });
        }
        Ok(Self {
            proprietary_id: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            proprietary_command: bytes[4],
            proprietary_data: &bytes[5..],
        })
    }
}

impl Serialize for ProprietaryCommand<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        FIXED_LEN + self.proprietary_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.proprietary_id.to_be_bytes());
        buf[4] = self.proprietary_command;
        buf[5..need].copy_from_slice(self.proprietary_data);
        Ok(need)
    }
}

impl<'a> OperationDef<'a> for ProprietaryCommand<'a> {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "PROPRIETARY_COMMAND";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = ProprietaryCommand {
            proprietary_id: 0xDEAD_BEEF,
            proprietary_command: 0x01,
            proprietary_data: &[0xAA, 0xBB, 0xCC],
        };
        let bytes = op.to_bytes();
        let back = ProprietaryCommand::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn empty_data_round_trip() {
        let op = ProprietaryCommand {
            proprietary_id: 0x1234_5678,
            proprietary_command: 0xFF,
            proprietary_data: &[],
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), FIXED_LEN);
        let back = ProprietaryCommand::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = ProprietaryCommand {
            proprietary_id: 1,
            proprietary_command: 0,
            proprietary_data: &[0x01],
        };
        let bytes = op.to_bytes();
        let mut op2 = op.clone();
        op2.proprietary_id = 999;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
