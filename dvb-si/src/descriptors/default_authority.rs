//! Default Authority Descriptor — ETSI TS 102 323 §12.2.1 (tag 0x73).
//!
//! Carried inside NIT, BAT, SDT, EIT to provide the default TV-Anytime
//! authority prefix used when resolving relative CRIDs.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for default_authority_descriptor.
pub const TAG: u8 = 0x73;
const HEADER_LEN: usize = 2;

/// Default Authority Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DefaultAuthorityDescriptor<'a> {
    /// Raw ASCII default authority bytes (e.g. b"bbc.co.uk").
    pub default_authority: &'a [u8],
}

impl<'a> Parse<'a> for DefaultAuthorityDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DefaultAuthorityDescriptor",
            "unexpected tag for default authority descriptor",
        )?;
        Ok(Self {
            default_authority: body,
        })
    }
}

impl Serialize for DefaultAuthorityDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.default_authority.len()
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
        buf[1] = self.default_authority.len() as u8;
        buf[HEADER_LEN..len].copy_from_slice(self.default_authority);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for DefaultAuthorityDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DEFAULT_AUTHORITY";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_authority_bytes() {
        let bytes = [TAG, 9, b'b', b'b', b'c', b'.', b'c', b'o', b'.', b'u', b'k'];
        let d = DefaultAuthorityDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.default_authority, b"bbc.co.uk");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            DefaultAuthorityDescriptor::parse(&[0x7A, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x7A, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_header() {
        assert!(matches!(
            DefaultAuthorityDescriptor::parse(&[TAG]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let bytes = [TAG, 5, 1, 2, 3];
        assert!(matches!(
            DefaultAuthorityDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn empty_authority_is_valid() {
        let bytes = [TAG, 0];
        let d = DefaultAuthorityDescriptor::parse(&bytes).unwrap();
        assert!(d.default_authority.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = DefaultAuthorityDescriptor {
            default_authority: b"example.com",
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(DefaultAuthorityDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = DefaultAuthorityDescriptor {
            default_authority: b"test",
        };
        let mut buf = vec![0u8; 1];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }
}
