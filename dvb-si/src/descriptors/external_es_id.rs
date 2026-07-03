//! External_ES_ID Descriptor — ISO/IEC 13818-1 §2.6.46, Table 2-78 (tag 0x20).
//!
//! Carries a single 16-bit External_ES_ID.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for External_ES_ID_descriptor.
pub const TAG: u8 = 0x20;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 2;

/// External_ES_ID Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExternalEsIdDescriptor {
    /// External elementary stream ID.
    pub external_es_id: u16,
}

impl<'a> Parse<'a> for ExternalEsIdDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ExternalEsIdDescriptor",
            "unexpected tag for External_ES_ID_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "External_ES_ID_descriptor length must equal 2",
            });
        }
        Ok(Self {
            external_es_id: u16::from_be_bytes([body[0], body[1]]),
        })
    }
}

impl Serialize for ExternalEsIdDescriptor {
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
        buf[HEADER_LEN..HEADER_LEN + 2].copy_from_slice(&self.external_es_id.to_be_bytes());
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for ExternalEsIdDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "EXTERNAL_ES_ID";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_es_id() {
        let d = ExternalEsIdDescriptor::parse(&[TAG, 2, 0xAB, 0xCD]).unwrap();
        assert_eq!(d.external_es_id, 0xABCD);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = ExternalEsIdDescriptor::parse(&[0x02, 2, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = ExternalEsIdDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = ExternalEsIdDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = ExternalEsIdDescriptor {
            external_es_id: 0xDEAD,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(ExternalEsIdDescriptor::parse(&buf).unwrap(), d);
    }
}
