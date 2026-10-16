//! IP/MAC_platform_name_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.2
//! Table 20 (tag 0x0C).
//!
//! Gives the name of the IP/MAC platform in one language: a 24-bit
//! `ISO_639_language_code` followed by `text_char`s coded per EN 300 468
//! Annex A. Allowed in the INT platform loop only (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for IP/MAC_platform_name_descriptor.
pub const TAG: u8 = 0x0C;
const HEADER_LEN: usize = 2;
const LANG_LEN: usize = 3;

/// IP/MAC_platform_name_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct IpMacPlatformNameDescriptor<'a> {
    /// `ISO_639_language_code` of the platform name.
    pub language_code: LangCode,
    /// `text_char`s — the platform name (EN 300 468 Annex A).
    pub text: DvbText<'a>,
}

impl<'a> Parse<'a> for IpMacPlatformNameDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "IpMacPlatformNameDescriptor",
            "unexpected tag for IP/MAC_platform_name_descriptor",
        )?;
        let (lang, text) = body
            .split_first_chunk::<LANG_LEN>()
            .ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "IP/MAC_platform_name_descriptor shorter than ISO_639_language_code",
            })?;
        Ok(Self {
            language_code: LangCode(*lang),
            text: DvbText::new(text),
        })
    }
}

impl Serialize for IpMacPlatformNameDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + LANG_LEN + self.text.len()
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
        buf[2..5].copy_from_slice(&self.language_code.0);
        buf[5..len].copy_from_slice(self.text.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for IpMacPlatformNameDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "IP_MAC_PLATFORM_NAME";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: language "foo", text "Bar Text".
        let mut bytes = vec![TAG, 11, b'f', b'o', b'o'];
        bytes.extend_from_slice(b"Bar Text");
        let d = IpMacPlatformNameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.language_code, LangCode(*b"foo"));
        assert_eq!(d.text.raw(), b"Bar Text");
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn rejects_short_body_and_wrong_tag() {
        assert!(IpMacPlatformNameDescriptor::parse(&[TAG, 2, b'f', b'o']).is_err());
        assert!(IpMacPlatformNameDescriptor::parse(&[0x0D, 3, b'f', b'o', b'o']).is_err());
    }
}
