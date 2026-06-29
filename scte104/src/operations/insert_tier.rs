//! insert_tier_data() — ANSI/SCTE 104 2023 §9.8.9, Table 9-31 (opID 0x010F).
//!
//! Supplemental usage. Sets the tier field in the resulting SCTE 35 section.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for insert_tier_data (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x010F;

/// Fixed wire length.
pub const LEN: usize = 2;

/// insert_tier_data() — §9.8.9, Table 9-31.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertTier {
    /// `tier_data` — 2 bytes, upper 4 bits = 0, lower 12 bits = tier value.
    pub tier_data: u16,
}

impl<'a> Parse<'a> for InsertTier {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < LEN {
            return Err(Error::BufferTooShort {
                need: LEN,
                have: bytes.len(),
                what: "insert_tier_data",
            });
        }
        Ok(Self {
            tier_data: u16::from_be_bytes([bytes[0], bytes[1]]),
        })
    }
}

impl Serialize for InsertTier {
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
        buf[0..2].copy_from_slice(&self.tier_data.to_be_bytes());
        Ok(LEN)
    }
}

impl<'a> OperationDef<'a> for InsertTier {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_TIER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = InsertTier { tier_data: 0x0FFF };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), LEN);
        let back = InsertTier::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InsertTier { tier_data: 0x0FFF };
        let bytes = op.to_bytes();
        let mut op2 = op;
        op2.tier_data = 0x0001;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
