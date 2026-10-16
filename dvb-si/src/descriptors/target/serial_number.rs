//! target_serial_number_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.4
//! Table 22 (INT) / ETSI TS 102 006 §9.5.2.5 Table 24 (UNT) (tag 0x08).
//!
//! Targets an action at a specific receiver. `serial_data_byte` is "intended
//! to target devices based on some manufacturing id. No further definition on
//! the semantics of this information is implied" — an opaque manufacturer id.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for target_serial_number_descriptor.
pub const TAG: u8 = 0x08;
const HEADER_LEN: usize = 2;

/// target_serial_number_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetSerialNumberDescriptor<'a> {
    /// `serial_data_byte`s — opaque manufacturer-defined device identifier.
    pub serial_data: &'a [u8],
}

impl<'a> Parse<'a> for TargetSerialNumberDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let serial_data = descriptor_body(
            bytes,
            TAG,
            "TargetSerialNumberDescriptor",
            "unexpected tag for target_serial_number_descriptor",
        )?;
        Ok(Self { serial_data })
    }
}

impl Serialize for TargetSerialNumberDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.serial_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[HEADER_LEN..len].copy_from_slice(self.serial_data);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetSerialNumberDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_SERIAL_NUMBER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: serial 12 34 56 78, and the empty descriptor.
        for bytes in [&[TAG, 4, 0x12, 0x34, 0x56, 0x78][..], &[TAG, 0][..]] {
            let d = TargetSerialNumberDescriptor::parse(bytes).unwrap();
            assert_eq!(d.serial_data, &bytes[2..]);
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn rejects_wrong_tag_and_overrun() {
        assert!(TargetSerialNumberDescriptor::parse(&[0x06, 0]).is_err());
        assert!(TargetSerialNumberDescriptor::parse(&[TAG, 5, 1]).is_err());
    }
}
