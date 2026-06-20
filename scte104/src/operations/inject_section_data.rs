//! inject_section_data_request() — ANSI/SCTE 104 2023 §9.8.3, Table 9-25 (opID 0x0100).
//!
//! Normal usage request. Generates an SCTE 35 section directly from a binary
//! image of the SCTE 35 command contents (following splice_command_type).

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use dvb_common::{Parse, Serialize};

/// `opID` for inject_section_data_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0100;

/// inject_section_data_request() — §9.8.3, Table 9-25.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InjectSectionData<'a> {
    /// `SCTE35_command_length` — 2 bytes.
    pub scte35_command_length: u16,
    /// `SCTE35_protocol_version` — 1 byte.
    pub scte35_protocol_version: u8,
    /// `SCTE35_command_type` — 1 byte.
    pub scte35_command_type: u8,
    /// `SCTE35_command_contents()` — variable, binary image of the SCTE 35
    /// command body (following the command_type, up to but not including
    /// descriptor_loop_length).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub scte35_command_contents: &'a [u8],
}

/// Fixed-size portion before command_contents.
pub const FIXED_LEN: usize = 4;

impl<'a> Parse<'a> for InjectSectionData<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN,
                have: bytes.len(),
                what: "inject_section_data header",
            });
        }
        let scte35_command_length = u16::from_be_bytes([bytes[0], bytes[1]]);
        let scte35_protocol_version = bytes[2];
        let scte35_command_type = bytes[3];
        let contents_len = scte35_command_length as usize;
        if bytes.len() < FIXED_LEN + contents_len {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN + contents_len,
                have: bytes.len(),
                what: "inject_section_data contents",
            });
        }
        Ok(Self {
            scte35_command_length,
            scte35_protocol_version,
            scte35_command_type,
            scte35_command_contents: &bytes[FIXED_LEN..FIXED_LEN + contents_len],
        })
    }
}

impl Serialize for InjectSectionData<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        FIXED_LEN + self.scte35_command_contents.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..2].copy_from_slice(&self.scte35_command_length.to_be_bytes());
        buf[2] = self.scte35_protocol_version;
        buf[3] = self.scte35_command_type;
        buf[FIXED_LEN..need].copy_from_slice(self.scte35_command_contents);
        Ok(need)
    }
}

impl<'a> OperationDef<'a> for InjectSectionData<'a> {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INJECT_SECTION_DATA";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = InjectSectionData {
            scte35_command_length: 4,
            scte35_protocol_version: 0,
            scte35_command_type: 0x05,
            scte35_command_contents: &[0xAA, 0xBB, 0xCC, 0xDD],
        };
        let bytes = op.to_bytes();
        let back = InjectSectionData::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InjectSectionData {
            scte35_command_length: 2,
            scte35_protocol_version: 0,
            scte35_command_type: 0,
            scte35_command_contents: &[0x01, 0x02],
        };
        let bytes = op.to_bytes();
        let mut op2 = op.clone();
        op2.scte35_command_type = 0xFF;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
