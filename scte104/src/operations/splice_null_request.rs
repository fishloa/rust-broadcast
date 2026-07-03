//! splice_null_request_data() — ANSI/SCTE 104 2023 §9.8.2, Table 9-24 (opID 0x0102).
//!
//! Normal usage request. Generates an SCTE 35 splice_null operation. Empty body.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for splice_null_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0102;

/// splice_null_request_data() — §9.8.2, Table 9-24.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceNullRequest;

impl<'a> Parse<'a> for SpliceNullRequest {
    type Error = Error;
    fn parse(_bytes: &'a [u8]) -> Result<Self> {
        Ok(Self)
    }
}

impl Serialize for SpliceNullRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        0
    }
    fn serialize_into(&self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl OperationDef<'_> for SpliceNullRequest {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "SPLICE_NULL_REQUEST";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = SpliceNullRequest;
        assert_eq!(op.serialized_len(), 0);
        let bytes = op.to_bytes();
        assert!(bytes.is_empty());
        assert_eq!(SpliceNullRequest::parse(&bytes).unwrap(), op);
    }
}
