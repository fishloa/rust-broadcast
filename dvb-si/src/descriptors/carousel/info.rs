//! Info Descriptor — ETSI EN 301 192 V1.7.1 §10.2.4, Table 52 (data-carousel
//! tag 0x03). PDF pp.65-66, transcribed in `docs/descriptors/en_301_192/52-*.md`,
//! verified against the rendered pages.
//!
//! A plain-text description of a module or group: `ISO_639_language_code`(24)
//! then the `text_char` string (EN 300 468 Annex A character sets).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for info_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x03;
const HEADER_LEN: usize = 2;
const LANG_LEN: usize = 3;

/// Info Descriptor (carousel tag 0x03).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InfoDescriptor<'a> {
    /// `ISO_639_language_code` — language of `text`.
    pub language_code: LangCode,
    /// `text_char` string — description of the module or group.
    pub text: DvbText<'a>,
}

impl<'a> Parse<'a> for InfoDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "InfoDescriptor",
            "unexpected tag for info_descriptor",
        )?;
        let (lang, text) =
            body.split_first_chunk::<LANG_LEN>()
                .ok_or(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "info_descriptor body shorter than ISO_639_language_code",
                })?;
        Ok(Self {
            language_code: LangCode(*lang),
            text: DvbText::new(text),
        })
    }
}

impl Serialize for InfoDescriptor<'_> {
    type Error = crate::error::Error;
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
        buf[HEADER_LEN..HEADER_LEN + LANG_LEN].copy_from_slice(&self.language_code.0);
        buf[HEADER_LEN + LANG_LEN..len].copy_from_slice(self.text.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for InfoDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "INFO";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let bytes = [TAG, 8, b'e', b'n', b'g', b'h', b'e', b'l', b'l', b'o'];
        let d = InfoDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.language_code, LangCode(*b"eng"));
        assert_eq!(d.text.raw(), b"hello");
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert_eq!(InfoDescriptor::parse(&out).unwrap(), d);
    }

    #[test]
    fn empty_text_short_body_and_overflow() {
        let bytes = [TAG, 3, b'f', b'r', b'a'];
        let d = InfoDescriptor::parse(&bytes).unwrap();
        assert!(d.text.is_empty());
        assert!(InfoDescriptor::parse(&[TAG, 2, b'e', b'n']).is_err());
        let big = vec![b'x'; 253];
        let d = InfoDescriptor {
            language_code: LangCode(*b"eng"),
            text: DvbText::new(&big),
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
