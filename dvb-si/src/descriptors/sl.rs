//! SL Descriptor — ISO/IEC 13818-1 §2.6.42, Table 2-76 (tag 0x1E).
//!
//! Carries a single 16-bit ES_ID.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for SL_descriptor.
pub const TAG: u8 = 0x1E;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 2;

/// SL Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SlDescriptor {
    /// Elementary stream ID.
    pub es_id: u16,
}

impl<'a> Parse<'a> for SlDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SlDescriptor",
            "unexpected tag for SL_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "SL_descriptor length must equal 2",
            });
        }
        Ok(Self {
            es_id: u16::from_be_bytes([body[0], body[1]]),
        })
    }
}

impl Serialize for SlDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN as usize
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
        buf[HEADER_LEN..HEADER_LEN + 2].copy_from_slice(&self.es_id.to_be_bytes());
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for SlDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SL";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_es_id() {
        let d = SlDescriptor::parse(&[TAG, 2, 0x12, 0x34]).unwrap();
        assert_eq!(d.es_id, 0x1234);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = SlDescriptor::parse(&[0x02, 2, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = SlDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = SlDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = SlDescriptor { es_id: 0xBEEF };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(SlDescriptor::parse(&buf).unwrap(), d);
    }
}
