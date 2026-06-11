//! Registration Descriptor — ISO/IEC 13818-1 §2.6.13 (tag 0x05).
//!
//! Identifies the registration of a stream or program via a 4-byte
//! `format_identifier`. Optionally followed by additional identification
//! information.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for registration_descriptor.
pub const TAG: u8 = 0x05;
const HEADER_LEN: usize = 2;
const FORMAT_IDENTIFIER_LEN: usize = 4;

/// Registration Descriptor.
///
/// The `format_identifier` is a 4-byte code registered with a standards body
/// (e.g. SMPTE, ATSC) that identifies the coding format of the associated
/// stream. Any bytes beyond the 4-byte identifier are treated as opaque
/// additional identification info.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct RegistrationDescriptor<'a> {
    /// 4-byte format identifier (e.g. b"AC-3", b"HDMV", b"dtsx").
    pub format_identifier: [u8; FORMAT_IDENTIFIER_LEN],
    /// Optional additional identification bytes following the format identifier.
    pub additional_identification_info: &'a [u8],
}

impl<'a> Parse<'a> for RegistrationDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "RegistrationDescriptor",
            "unexpected tag for registration_descriptor",
        )?;
        if body.len() < FORMAT_IDENTIFIER_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "registration_descriptor length too short for format_identifier",
            });
        }
        let mut format_identifier = [0u8; FORMAT_IDENTIFIER_LEN];
        format_identifier.copy_from_slice(&body[..FORMAT_IDENTIFIER_LEN]);
        let additional_identification_info = &body[FORMAT_IDENTIFIER_LEN..];
        Ok(Self {
            format_identifier,
            additional_identification_info,
        })
    }
}

impl Serialize for RegistrationDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + FORMAT_IDENTIFIER_LEN + self.additional_identification_info.len()
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
        buf[1] = (len - HEADER_LEN) as u8;
        buf[HEADER_LEN..HEADER_LEN + FORMAT_IDENTIFIER_LEN]
            .copy_from_slice(&self.format_identifier);
        buf[HEADER_LEN + FORMAT_IDENTIFIER_LEN..len]
            .copy_from_slice(self.additional_identification_info);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for RegistrationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "REGISTRATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_format_identifier_only() {
        let bytes = [TAG, 4, b'A', b'C', b'-', b'3'];
        let d = RegistrationDescriptor::parse(&bytes).unwrap();
        assert_eq!(&d.format_identifier, b"AC-3");
        assert!(d.additional_identification_info.is_empty());
    }

    #[test]
    fn parse_with_additional_info() {
        let bytes = [TAG, 6, b'H', b'D', b'M', b'V', 0x01, 0x02];
        let d = RegistrationDescriptor::parse(&bytes).unwrap();
        assert_eq!(&d.format_identifier, b"HDMV");
        assert_eq!(d.additional_identification_info, &[0x01, 0x02]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = RegistrationDescriptor::parse(&[0x06, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x06, .. }));
    }

    #[test]
    fn parse_rejects_short_header() {
        let err = RegistrationDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_length_too_short_for_format_id() {
        let bytes = [TAG, 3, 0, 0, 0];
        let err = RegistrationDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_rejects_length_overflow() {
        let bytes = [TAG, 10, 0, 0, 0, 0];
        let err = RegistrationDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = RegistrationDescriptor {
            format_identifier: *b"dtsx",
            additional_identification_info: &[0xAA, 0xBB],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = RegistrationDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = RegistrationDescriptor {
            format_identifier: *b"AC-3",
            additional_identification_info: &[],
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = RegistrationDescriptor {
            format_identifier: *b"AC-3",
            additional_identification_info: &[0x01],
        };
        assert_eq!(d.serialized_len() - 2, 5);
    }
}
