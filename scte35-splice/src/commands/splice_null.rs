//! splice_null() — ANSI/SCTE 35 2023r1 §9.7.1, Table 8 (splice_command_type 0x00).
//!
//! An empty command body; lets a section carry descriptors with no other
//! command, or act as a heartbeat. Parses/serializes to zero body bytes.

use crate::error::{Error, Result};
use crate::traits::CommandDef;
use dvb_common::{Parse, Serialize};

/// `splice_command_type` for splice_null (§9.6.1, Table 7).
pub const COMMAND_TYPE: u8 = 0x00;

/// splice_null() command (empty body).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceNull;

impl<'a> Parse<'a> for SpliceNull {
    type Error = Error;
    fn parse(_bytes: &'a [u8]) -> Result<Self> {
        Ok(Self)
    }
}

impl Serialize for SpliceNull {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        0
    }
    fn serialize_into(&self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl<'a> CommandDef<'a> for SpliceNull {
    const COMMAND_TYPE: u8 = COMMAND_TYPE;
    const NAME: &'static str = "SPLICE_NULL";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_empty() {
        let cmd = SpliceNull;
        assert_eq!(cmd.serialized_len(), 0);
        let bytes = cmd.to_bytes();
        assert!(bytes.is_empty());
        assert_eq!(SpliceNull::parse(&bytes).unwrap(), cmd);
    }
}
