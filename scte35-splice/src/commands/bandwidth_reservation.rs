//! bandwidth_reservation() — ANSI/SCTE 35 2023r1 §9.7.5, Table 12
//! (splice_command_type 0x07).
//!
//! An empty command body, distinct from splice_null() so receivers can handle
//! it uniquely (e.g. remove it from the multiplex).

use crate::error::{Error, Result};
use crate::traits::CommandDef;
use dvb_common::{Parse, Serialize};

/// `splice_command_type` for bandwidth_reservation (§9.6.1, Table 7).
pub const COMMAND_TYPE: u8 = 0x07;

/// bandwidth_reservation() command (empty body).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BandwidthReservation;

impl<'a> Parse<'a> for BandwidthReservation {
    type Error = Error;
    fn parse(_bytes: &'a [u8]) -> Result<Self> {
        Ok(Self)
    }
}

impl Serialize for BandwidthReservation {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        0
    }
    fn serialize_into(&self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl<'a> CommandDef<'a> for BandwidthReservation {
    const COMMAND_TYPE: u8 = COMMAND_TYPE;
    const NAME: &'static str = "BANDWIDTH_RESERVATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_empty() {
        let cmd = BandwidthReservation;
        assert_eq!(cmd.serialized_len(), 0);
        assert_eq!(BandwidthReservation::parse(&cmd.to_bytes()).unwrap(), cmd);
    }
}
