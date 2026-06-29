//! Control Word operations — ANSI/SCTE 104 2023 §9.4.3, §9.4.4 (opIDs 0x0300, 0x0301).
//!
//! `delete_ControlWord_data()`: deletes a Control Word from the Injector's
//! database. `update_ControlWord_data()`: sets three 64-bit Control Words
//! (CW_A, CW_B, CW_C) for a CW index.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for delete_ControlWord_data (§8.3, Table 8-4).
pub const DELETE_OP_ID: u16 = 0x0300;

/// `opID` for update_ControlWord_data (§8.3, Table 8-4).
pub const UPDATE_OP_ID: u16 = 0x0301;

/// Fixed wire length of `delete_ControlWord_data()`.
pub const DELETE_LEN: usize = 1;

/// Fixed wire length of `update_ControlWord_data()`.
pub const UPDATE_LEN: usize = 1 + 8 + 8 + 8; // CW_index + CW_A + CW_B + CW_C = 25

/// delete_ControlWord_data() — §9.4.4, Table 9-10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DeleteControlWord {
    /// `CW_index` — 1 byte (0-255).
    pub cw_index: u8,
}

/// update_ControlWord_data() — §9.4.3, Table 9-9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UpdateControlWord {
    /// `CW_index` — 1 byte (0-255).
    pub cw_index: u8,
    /// `CW_A` — 8 bytes (64-bit big-endian).
    pub cw_a: u64,
    /// `CW_B` — 8 bytes (64-bit big-endian).
    pub cw_b: u64,
    /// `CW_C` — 8 bytes (64-bit big-endian).
    pub cw_c: u64,
}

impl<'a> Parse<'a> for DeleteControlWord {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < DELETE_LEN {
            return Err(Error::BufferTooShort {
                need: DELETE_LEN,
                have: bytes.len(),
                what: "delete_ControlWord data",
            });
        }
        Ok(Self { cw_index: bytes[0] })
    }
}

impl Serialize for DeleteControlWord {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DELETE_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < DELETE_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: DELETE_LEN,
                have: buf.len(),
            });
        }
        buf[0] = self.cw_index;
        Ok(DELETE_LEN)
    }
}

impl<'a> OperationDef<'a> for DeleteControlWord {
    const OP_ID: u16 = DELETE_OP_ID;
    const NAME: &'static str = "DELETE_CONTROL_WORD";
}

impl<'a> Parse<'a> for UpdateControlWord {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < UPDATE_LEN {
            return Err(Error::BufferTooShort {
                need: UPDATE_LEN,
                have: bytes.len(),
                what: "update_ControlWord data",
            });
        }
        Ok(Self {
            cw_index: bytes[0],
            cw_a: u64::from_be_bytes([
                bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
            ]),
            cw_b: u64::from_be_bytes([
                bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
                bytes[16],
            ]),
            cw_c: u64::from_be_bytes([
                bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22], bytes[23],
                bytes[24],
            ]),
        })
    }
}

impl Serialize for UpdateControlWord {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        UPDATE_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < UPDATE_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: UPDATE_LEN,
                have: buf.len(),
            });
        }
        buf[0] = self.cw_index;
        buf[1..9].copy_from_slice(&self.cw_a.to_be_bytes());
        buf[9..17].copy_from_slice(&self.cw_b.to_be_bytes());
        buf[17..25].copy_from_slice(&self.cw_c.to_be_bytes());
        Ok(UPDATE_LEN)
    }
}

impl<'a> OperationDef<'a> for UpdateControlWord {
    const OP_ID: u16 = UPDATE_OP_ID;
    const NAME: &'static str = "UPDATE_CONTROL_WORD";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_round_trip() {
        let op = DeleteControlWord { cw_index: 5 };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), DELETE_LEN);
        let back = DeleteControlWord::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn update_round_trip() {
        let op = UpdateControlWord {
            cw_index: 0,
            cw_a: 0xDEAD_BEEF_CAFE_BABE,
            cw_b: 0,
            cw_c: 0,
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), UPDATE_LEN);
        let back = UpdateControlWord::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn delete_mutate_field_changes_output() {
        let op = DeleteControlWord { cw_index: 5 };
        let bytes = op.to_bytes();
        let mut op2 = op;
        op2.cw_index = 99;
        assert_ne!(op2.to_bytes(), bytes);
    }

    #[test]
    fn update_mutate_field_changes_output() {
        let op = UpdateControlWord::default();
        let bytes = op.to_bytes();
        let mut op2 = op;
        op2.cw_a = 1;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
