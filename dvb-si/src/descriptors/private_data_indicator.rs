//! Private Data Indicator Descriptor — ISO/IEC 13818-1 §2.6.22 (tag 0x0F).
//!
//! Carries a 4-byte private data specifier that identifies the organization
//! or entity that defined the private data carried in the associated stream.

use super::descriptor_body;
pub use super::private_data_specifier::private_data_specifier_name;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for private_data_indicator_descriptor.
pub const TAG: u8 = 0x0F;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 4;

/// Private Data Indicator Descriptor.
///
/// The `private_data_specifier` is a 4-byte value assigned by a standards body
/// or industry consortium to identify the organization that defined the private
/// data format used in the associated elementary stream.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PrivateDataIndicatorDescriptor {
    /// 4-byte private data specifier identifier.
    pub private_data_specifier: [u8; 4],
}

impl<'a> Parse<'a> for PrivateDataIndicatorDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "PrivateDataIndicatorDescriptor",
            "unexpected tag for private_data_indicator_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "private_data_indicator_descriptor length must equal 4",
            });
        }
        let mut private_data_specifier = [0u8; 4];
        private_data_specifier.copy_from_slice(&body[..4]);
        Ok(Self {
            private_data_specifier,
        })
    }
}

impl Serialize for PrivateDataIndicatorDescriptor {
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
        buf[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&self.private_data_specifier);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for PrivateDataIndicatorDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "PRIVATE_DATA_INDICATOR";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_private_data_specifier() {
        let bytes = [TAG, 4, 0x00, 0x00, 0x00, 0x01];
        let d = PrivateDataIndicatorDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.private_data_specifier, [0x00, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = PrivateDataIndicatorDescriptor::parse(&[0x10, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x10, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        // Length says 3 but valid is 4 — buffer is large enough to read header+3
        let bytes = [TAG, 3, 0, 0, 0, 0];
        let err = PrivateDataIndicatorDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = PrivateDataIndicatorDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = PrivateDataIndicatorDescriptor {
            private_data_specifier: [0xAA, 0xBB, 0xCC, 0xDD],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = PrivateDataIndicatorDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = PrivateDataIndicatorDescriptor {
            private_data_specifier: [0x00, 0x00, 0x00, 0x01],
        };
        assert_eq!(d.serialized_len() - 2, 4);
    }
}
