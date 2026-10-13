//! System Clock Descriptor — ISO/IEC 13818-1 §2.6.20 (tag 0x0B).
//!
//! Describes the system clock accuracy.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for system_clock_descriptor.
pub const TAG: u8 = 0x0B;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 2;
/// `reserved(1)` after `external_clock_reference_indicator` (Table 2-64),
/// emitted as 1 (r03-W7; confirmed by the TSDuck oracle PMT).
const RESERVED_1_BIT: u8 = 0x40;
/// Trailing `reserved(5)`, emitted as `11111`.
const RESERVED_5_BITS: u8 = 0x1F;

/// System Clock Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SystemClockDescriptor {
    /// External clock reference indicator.
    pub external_clock_reference_indicator: bool,
    /// Clock accuracy integer (6 bits).
    pub clock_accuracy_integer: u8,
    /// Clock accuracy exponent (3 bits).
    pub clock_accuracy_exponent: u8,
}

impl<'a> Parse<'a> for SystemClockDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SystemClockDescriptor",
            "unexpected tag for system_clock_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "system_clock_descriptor length must equal 2",
            });
        }
        let b0 = body[0];
        let external_clock_reference_indicator = (b0 & 0x80) != 0;
        let clock_accuracy_integer = b0 & 0x3F;
        let b1 = body[1];
        let clock_accuracy_exponent = b1 >> 5;
        Ok(Self {
            external_clock_reference_indicator,
            clock_accuracy_integer,
            clock_accuracy_exponent,
        })
    }
}

impl Serialize for SystemClockDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN] = ((self.external_clock_reference_indicator as u8) << 7)
            | RESERVED_1_BIT
            | (self.clock_accuracy_integer & 0x3F);
        buf[HEADER_LEN + 1] = ((self.clock_accuracy_exponent & 0x07) << 5) | RESERVED_5_BITS;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for SystemClockDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SYSTEM_CLOCK";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            2,
            0b1011_1111, // external=1, reserved=0, integer=63
            0b101_00000, // exponent=5, reserved=0
        ];
        let d = SystemClockDescriptor::parse(&bytes).unwrap();
        assert!(d.external_clock_reference_indicator);
        assert_eq!(d.clock_accuracy_integer, 63);
        assert_eq!(d.clock_accuracy_exponent, 5);
    }

    #[test]
    fn serialize_round_trip() {
        let d = SystemClockDescriptor {
            external_clock_reference_indicator: false,
            clock_accuracy_integer: 42,
            clock_accuracy_exponent: 2,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = SystemClockDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = SystemClockDescriptor::parse(&[0x0C, 2, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x0C, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = SystemClockDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = SystemClockDescriptor {
            external_clock_reference_indicator: false,
            clock_accuracy_integer: 0,
            clock_accuracy_exponent: 0,
        };
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    /// r03-W7 — H.222.0 Table 2-64 (ISO/IEC 13818-1:2021). Evidence for reserved=1 is the TSDuck-compiled oracle
    /// PMT `tsduck-mpeg-simple-pmt.bin` (TSDuck only; no real broadcast capture carries it). (1) conformant bytes (reserved=1) round-trip byte-exactly;
    /// (2) the same descriptor with reserved=0 (non-conformant sender) parses to
    /// equal typed fields and re-serializes to the normalized reserved=1 form.
    #[test]
    fn reserved_bits_one_on_serialize_and_zero_input_normalized() {
        let conformant: &[u8] = &[TAG, 2, 0xFF, 0xBF];
        let zeroed: &[u8] = &[TAG, 2, 0xBF, 0xA0];
        let d = SystemClockDescriptor::parse(conformant).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, conformant);

        let dz = SystemClockDescriptor::parse(zeroed).unwrap();
        assert_eq!(dz, d, "typed fields must not depend on reserved bits");
        let mut outz = vec![0u8; dz.serialized_len()];
        dz.serialize_into(&mut outz).unwrap();
        assert_eq!(outz, conformant);
    }
}
