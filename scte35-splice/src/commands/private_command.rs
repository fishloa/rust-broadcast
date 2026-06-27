//! private_command() — ANSI/SCTE 35 2023r1 §9.7.6, Table 13
//! (splice_command_type 0xFF).
//!
//! A 32-bit registered `identifier` followed by an opaque `private_byte`
//! payload (the rest of the command body). The payload length is determined by
//! `splice_command_length` in the enclosing section, so this struct borrows the
//! whole remaining command body after the identifier.

use crate::error::{Error, Result};
use crate::traits::CommandDef;
use dvb_common::{Parse, Serialize};

/// `splice_command_type` for private_command (§9.6.1, Table 7).
pub const COMMAND_TYPE: u8 = 0xFF;

/// Bytes of the fixed `identifier` field.
const IDENTIFIER_LEN: usize = 4;

/// private_command() — §9.7.6, Table 13.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PrivateCommand<'a> {
    /// 32-bit registration identifier (SMPTE Registration Authority format).
    pub identifier: u32,
    /// The remaining `private_byte` payload, verbatim.
    pub private_bytes: &'a [u8],
}

impl<'a> Parse<'a> for PrivateCommand<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (id_bytes, private_bytes) =
            bytes
                .split_first_chunk::<IDENTIFIER_LEN>()
                .ok_or(Error::BufferTooShort {
                    need: IDENTIFIER_LEN,
                    have: bytes.len(),
                    what: "private_command identifier",
                })?;
        let identifier = u32::from_be_bytes(*id_bytes);
        Ok(Self {
            identifier,
            private_bytes,
        })
    }
}

impl Serialize for PrivateCommand<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        IDENTIFIER_LEN + self.private_bytes.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..IDENTIFIER_LEN].copy_from_slice(&self.identifier.to_be_bytes());
        buf[IDENTIFIER_LEN..need].copy_from_slice(self.private_bytes);
        Ok(need)
    }
}

impl<'a> CommandDef<'a> for PrivateCommand<'a> {
    const COMMAND_TYPE: u8 = COMMAND_TYPE;
    const NAME: &'static str = "PRIVATE_COMMAND";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let cmd = PrivateCommand {
            identifier: 0x43554549,
            private_bytes: &[0xDE, 0xAD, 0xBE, 0xEF],
        };
        let bytes = cmd.to_bytes();
        assert_eq!(&bytes[0..4], &[0x43, 0x55, 0x45, 0x49]);
        let back = PrivateCommand::parse(&bytes).unwrap();
        assert_eq!(cmd, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn empty_private_bytes() {
        let cmd = PrivateCommand {
            identifier: 0x01020304,
            private_bytes: &[],
        };
        let bytes = cmd.to_bytes();
        assert_eq!(bytes.len(), 4);
        assert_eq!(PrivateCommand::parse(&bytes).unwrap(), cmd);
    }

    #[test]
    fn rejects_short_identifier() {
        assert!(matches!(
            PrivateCommand::parse(&[0x00, 0x01]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }
}
