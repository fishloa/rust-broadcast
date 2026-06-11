//! ECM Repetition Rate Descriptor — ETSI EN 301 192 §9.9, Table 44 (tag 0x78).
//!
//! Carried in the PMT to advertise the maximum interval between ECMs for a
//! given CA system. Per en_301_192.md "Table 44 — ECM repetition rate
//! descriptor" (PDF p. 56) the body is: CA_system_ID(16) +
//! ECM_repetition_rate(16) + trailing private_data_byte run.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for ECM_repetition_rate_descriptor.
pub const TAG: u8 = 0x78;
const HEADER_LEN: usize = 2;
const FIXED_LEN: usize = 4;

/// ECM Repetition Rate Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct EcmRepetitionRateDescriptor<'a> {
    /// 16-bit CA_system_ID this rate applies to.
    pub ca_system_id: u16,
    /// 16-bit ECM_repetition_rate (max ms between successive ECMs).
    pub ecm_repetition_rate: u16,
    /// Trailing private_data bytes.
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for EcmRepetitionRateDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "EcmRepetitionRateDescriptor",
            "unexpected tag for ECM_repetition_rate_descriptor",
        )?;
        if body.len() < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "ECM_repetition_rate_descriptor body shorter than 4 bytes",
            });
        }
        let ca_system_id = u16::from_be_bytes([body[0], body[1]]);
        let ecm_repetition_rate = u16::from_be_bytes([body[2], body[3]]);
        let private_data = &body[FIXED_LEN..];
        Ok(Self {
            ca_system_id,
            ecm_repetition_rate,
            private_data,
        })
    }
}

impl Serialize for EcmRepetitionRateDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.private_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if FIXED_LEN + self.private_data.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "ECM_repetition_rate_descriptor body exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (FIXED_LEN + self.private_data.len()) as u8;
        buf[2..4].copy_from_slice(&self.ca_system_id.to_be_bytes());
        buf[4..6].copy_from_slice(&self.ecm_repetition_rate.to_be_bytes());
        buf[HEADER_LEN + FIXED_LEN..len].copy_from_slice(self.private_data);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for EcmRepetitionRateDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ECM_REPETITION_RATE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_private_data() {
        let bytes = [TAG, 4, 0x06, 0x48, 0x01, 0xF4];
        let d = EcmRepetitionRateDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ca_system_id, 0x0648);
        assert_eq!(d.ecm_repetition_rate, 500);
        assert!(d.private_data.is_empty());
    }

    #[test]
    fn parse_with_private_data() {
        let bytes = [TAG, 6, 0x05, 0x00, 0x00, 0xC8, 0xAA, 0xBB];
        let d = EcmRepetitionRateDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ca_system_id, 0x0500);
        assert_eq!(d.ecm_repetition_rate, 200);
        assert_eq!(d.private_data, &[0xAA, 0xBB]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            EcmRepetitionRateDescriptor::parse(&[0x77, 4, 0, 0, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x77, .. }
        ));
    }

    #[test]
    fn parse_rejects_body_too_short() {
        let bytes = [TAG, 3, 0, 0, 0];
        assert!(matches!(
            EcmRepetitionRateDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let bytes = [TAG, 6, 0, 0, 0, 0];
        assert!(matches!(
            EcmRepetitionRateDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = EcmRepetitionRateDescriptor {
            ca_system_id: 0x0B00,
            ecm_repetition_rate: 1000,
            private_data: &[0x01, 0x02],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(EcmRepetitionRateDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = EcmRepetitionRateDescriptor {
            ca_system_id: 0,
            ecm_repetition_rate: 0,
            private_data: &[],
        };
        let mut buf = vec![0u8; 3];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serializes_to_stable_json() {
        // Borrowed `&[u8]` cannot deserialize from a JSON number array, so we
        // assert the Serialize impl is wired and emits stable JSON.
        let d = EcmRepetitionRateDescriptor {
            ca_system_id: 0x0648,
            ecm_repetition_rate: 480,
            private_data: &[0xFE],
        };
        let j = serde_json::to_string(&d).unwrap();
        // Valid, re-parseable JSON (key order is map-defined, so we do not
        // assert byte-for-byte string stability).
        let _v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert!(j.contains("ca_system_id"));
    }
}
