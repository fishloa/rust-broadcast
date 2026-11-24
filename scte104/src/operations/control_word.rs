//! Control Word operations — ANSI/SCTE 104 2023 §9.4.3, §9.4.4 (opIDs 0x0300, 0x0301).
//!
//! `delete_ControlWord_data()`: deletes a Control Word from the Injector's
//! database. `update_ControlWord_data()`: sets three 64-bit Control Words
//! (CW_A, CW_B, CW_C) for a CW index.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// `opID` for delete_ControlWord_data (§8.3, Table 8-4).
pub const DELETE_OP_ID: u16 = 0x0300;

/// `opID` for update_ControlWord_data (§8.3, Table 8-4).
pub const UPDATE_OP_ID: u16 = 0x0301;

/// Fixed wire length of `delete_ControlWord_data()`.
pub const DELETE_LEN: usize = 1;

/// Fixed wire length of `update_ControlWord_data()`.
pub const UPDATE_LEN: usize = 1 + 8 + 8 + 8; // CW_index + CW_A + CW_B + CW_C = 25

// Field byte offsets within `update_ControlWord_data()` (ANSI/SCTE 104 2023 §9.4.4).
const CW_INDEX_OFFSET: usize = 0;
const CW_A_OFFSET: usize = 1;
const CW_B_OFFSET: usize = CW_A_OFFSET + core::mem::size_of::<u64>();
const CW_C_OFFSET: usize = CW_B_OFFSET + core::mem::size_of::<u64>();

fn read_u64_be(bytes: &[u8], offset: usize) -> u64 {
    let mut be = [0u8; core::mem::size_of::<u64>()];
    be.copy_from_slice(&bytes[offset..offset + core::mem::size_of::<u64>()]);
    u64::from_be_bytes(be)
}

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
        need_in(bytes, DELETE_LEN, "delete_ControlWord data")?;
        Ok(Self { cw_index: bytes[0] })
    }
}

impl Serialize for DeleteControlWord {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DELETE_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, DELETE_LEN)?;
        buf[CW_INDEX_OFFSET] = self.cw_index;
        Ok(DELETE_LEN)
    }
}

impl OperationDef<'_> for DeleteControlWord {
    const OP_ID: u16 = DELETE_OP_ID;
    const NAME: &'static str = "DELETE_CONTROL_WORD";
}

impl<'a> Parse<'a> for UpdateControlWord {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, UPDATE_LEN, "update_ControlWord data")?;
        Ok(Self {
            cw_index: bytes[CW_INDEX_OFFSET],
            cw_a: read_u64_be(bytes, CW_A_OFFSET),
            cw_b: read_u64_be(bytes, CW_B_OFFSET),
            cw_c: read_u64_be(bytes, CW_C_OFFSET),
        })
    }
}

impl Serialize for UpdateControlWord {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        UPDATE_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, UPDATE_LEN)?;
        buf[CW_INDEX_OFFSET] = self.cw_index;
        buf[1..9].copy_from_slice(&self.cw_a.to_be_bytes());
        buf[9..17].copy_from_slice(&self.cw_b.to_be_bytes());
        buf[17..25].copy_from_slice(&self.cw_c.to_be_bytes());
        Ok(UPDATE_LEN)
    }
}

impl OperationDef<'_> for UpdateControlWord {
    const OP_ID: u16 = UPDATE_OP_ID;
    const NAME: &'static str = "UPDATE_CONTROL_WORD";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_round_trip() {
        let op = DeleteControlWord { cw_index: 5 };
        let bytes = op.try_to_bytes().unwrap();
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
        let bytes = op.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), UPDATE_LEN);
        let back = UpdateControlWord::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn delete_mutate_field_changes_output() {
        let op = DeleteControlWord { cw_index: 5 };
        let bytes = op.try_to_bytes().unwrap();
        let mut op2 = op;
        op2.cw_index = 99;
        assert_ne!(op2.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn update_mutate_field_changes_output() {
        let op = UpdateControlWord::default();
        let bytes = op.try_to_bytes().unwrap();
        let mut op2 = op;
        op2.cw_a = 1;
        assert_ne!(op2.try_to_bytes().unwrap(), bytes);
    }
}
