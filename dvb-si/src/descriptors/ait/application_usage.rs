//! Application Usage Descriptor — ETSI TS 102 809 §5.3.5.5, Table 23
//! (AIT tag 0x16).
//!
//! Carried in the AIT common descriptor loop. Indicates the application's
//! usage type — Table 11 defines known values (e.g. 0x01 = Digital Text).

use crate::descriptors::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for application_usage_descriptor (AIT namespace).
pub const TAG: u8 = 0x16;
const HEADER_LEN: usize = 2;

/// Known `usage_type` values — ETSI TS 102 809 §5.2.11.1.1 Table 11.
pub const USAGE_TYPE_DIGITAL_TEXT: u8 = 0x01;

/// Application Usage Descriptor (AIT tag 0x16).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationUsageDescriptor {
    /// 8-bit usage_type (Table 11: 0x01 = Digital Text, 0x02..0x7F reserved,
    /// 0x80..0xFF platform-specific).
    pub usage_type: u8,
}

impl ApplicationUsageDescriptor {
    /// Returns the well-known name for `usage_type`, or `None` if the value
    /// is not recognised.
    #[must_use]
    pub fn usage_type_name(&self) -> Option<&'static str> {
        match self.usage_type {
            USAGE_TYPE_DIGITAL_TEXT => Some("Digital Text"),
            _ => None,
        }
    }
}

impl<'a> Parse<'a> for ApplicationUsageDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ApplicationUsageDescriptor",
            "unexpected tag for application_usage_descriptor",
        )?;
        if body.is_empty() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_usage_descriptor body shorter than minimum 1 byte",
            });
        }
        Ok(Self {
            usage_type: body[0],
        })
    }
}

impl Serialize for ApplicationUsageDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + 1
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
        buf[1] = 1;
        buf[2] = self.usage_type;
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ApplicationUsageDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "APPLICATION_USAGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_digital_text() {
        let bytes = [TAG, 1, USAGE_TYPE_DIGITAL_TEXT];
        let d = ApplicationUsageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.usage_type, USAGE_TYPE_DIGITAL_TEXT);
        assert_eq!(d.usage_type_name(), Some("Digital Text"));
    }

    #[test]
    fn parse_platform_specific() {
        let bytes = [TAG, 1, 0x80];
        let d = ApplicationUsageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.usage_type, 0x80);
        assert_eq!(d.usage_type_name(), None);
    }

    #[test]
    fn serialize_round_trip() {
        let d = ApplicationUsageDescriptor {
            usage_type: USAGE_TYPE_DIGITAL_TEXT,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ApplicationUsageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_byte_identical() {
        let bytes = [TAG, 1, 0x01];
        let d = ApplicationUsageDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }
}
