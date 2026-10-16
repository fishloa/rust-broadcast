//! target_smartcard_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.5 Table 23
//! (INT) / ETSI TS 102 006 §9.5.2.1 Table 20 (UNT) (tag 0x06).
//!
//! Targets a receiver by smart card: a 32-bit `super_CA_system_id` (DVB CA
//! identifier as per ETSI TS 103 197) followed by `private_data_byte`s that
//! carry the smart card numbers.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for target_smartcard_descriptor.
pub const TAG: u8 = 0x06;
const HEADER_LEN: usize = 2;
const SUPER_CA_SYSTEM_ID_LEN: usize = 4;

/// target_smartcard_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetSmartcardDescriptor<'a> {
    /// `super_CA_system_id` — DVB CA identifier (ETSI TS 103 197).
    pub super_ca_system_id: u32,
    /// `private_data_byte`s — the smart card numbers; an opaque
    /// operator-defined structure (spec §8.4.5.5 / §9.5.2.1).
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for TargetSmartcardDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "TargetSmartcardDescriptor",
            "unexpected tag for target_smartcard_descriptor",
        )?;
        let (id, private_data) =
            body.split_first_chunk::<SUPER_CA_SYSTEM_ID_LEN>()
                .ok_or(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "target_smartcard_descriptor shorter than super_CA_system_id",
                })?;
        Ok(Self {
            super_ca_system_id: u32::from_be_bytes(*id),
            private_data,
        })
    }
}

impl Serialize for TargetSmartcardDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + SUPER_CA_SYSTEM_ID_LEN + self.private_data.len()
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
        buf[2..6].copy_from_slice(&self.super_ca_system_id.to_be_bytes());
        buf[6..len].copy_from_slice(self.private_data);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TargetSmartcardDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_SMARTCARD";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fields_and_round_trip() {
        // TSDuck test-022: super_CA_system_id 0x12345678 + 45 89 32 84 21 54.
        let bytes = [
            TAG, 0x0A, 0x12, 0x34, 0x56, 0x78, 0x45, 0x89, 0x32, 0x84, 0x21, 0x54,
        ];
        let d = TargetSmartcardDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.super_ca_system_id, 0x1234_5678);
        assert_eq!(d.private_data, &[0x45, 0x89, 0x32, 0x84, 0x21, 0x54]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_private_data_round_trip() {
        let bytes = [TAG, 0x04, 0x98, 0x76, 0x54, 0x32];
        let d = TargetSmartcardDescriptor::parse(&bytes).unwrap();
        assert!(d.private_data.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn rejects_short_body_and_wrong_tag() {
        assert!(TargetSmartcardDescriptor::parse(&[TAG, 3, 1, 2, 3]).is_err());
        assert!(TargetSmartcardDescriptor::parse(&[0x07, 4, 1, 2, 3, 4]).is_err());
    }
}
