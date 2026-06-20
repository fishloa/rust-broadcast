//! encrypted_DPI_request_data() — ANSI/SCTE 104 2023 §9.4.2, Table 9-8 (opID 0x0107).
//!
//! Supplemental usage. Adds encryption to the associated DPI request.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use dvb_common::{Parse, Serialize};

/// `opID` for encrypted_DPI_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0107;

/// Fixed wire length of `encrypted_DPI_request_data()`.
pub const LEN: usize = 2;

/// encrypted_DPI_request_data() — §9.4.2, Table 9-8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EncryptedDpi {
    /// `encryption_algorithm` — 1 byte, 6-bit field per SCTE 35.
    pub encryption_algorithm: u8,
    /// `CW_index` — 1 byte, control word index (0-255).
    pub cw_index: u8,
}

impl<'a> Parse<'a> for EncryptedDpi {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < LEN {
            return Err(Error::BufferTooShort {
                need: LEN,
                have: bytes.len(),
                what: "encrypted_DPI_request_data",
            });
        }
        Ok(Self {
            encryption_algorithm: bytes[0],
            cw_index: bytes[1],
        })
    }
}

impl Serialize for EncryptedDpi {
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
        buf[0] = self.encryption_algorithm;
        buf[1] = self.cw_index;
        Ok(LEN)
    }
}

impl<'a> OperationDef<'a> for EncryptedDpi {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "ENCRYPTED_DPI";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = EncryptedDpi {
            encryption_algorithm: 1,
            cw_index: 5,
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), LEN);
        let back = EncryptedDpi::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = EncryptedDpi {
            encryption_algorithm: 1,
            cw_index: 5,
        };
        let bytes = op.to_bytes();
        let mut op2 = op;
        op2.cw_index = 99;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
