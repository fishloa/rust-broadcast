//! SSU_event_name_descriptor — ETSI TS 102 006 §9.5.2.11 Table 34 (UNT tag
//! 0x05).
//!
//! Informs the user of the system software update as an event; similar in
//! structure and meaning to the short_event_descriptor: a language code, a
//! length-prefixed `name_char` string and a length-prefixed `text_char`
//! string (EN 300 468 Annex A). UNT common and operational loops (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for SSU_event_name_descriptor.
pub const TAG: u8 = 0x05;
const HEADER_LEN: usize = 2;
const LANG_LEN: usize = 3;

/// SSU_event_name_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SsuEventNameDescriptor<'a> {
    /// `ISO_639_language_code` of the name and text.
    pub language_code: LangCode,
    /// `name_char`s — the SSU event name.
    pub name: DvbText<'a>,
    /// `text_char`s — the text message.
    pub text: DvbText<'a>,
    /// Bytes after `text` inside `descriptor_length`; the spec defines none
    /// (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for SsuEventNameDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SsuEventNameDescriptor",
            "unexpected tag for SSU_event_name_descriptor",
        )?;
        let bad = |reason| Error::InvalidDescriptor { tag: TAG, reason };
        if body.len() < LANG_LEN + 2 {
            return Err(bad("SSU_event_name_descriptor body shorter than 5 bytes"));
        }
        let name_len = usize::from(body[LANG_LEN]);
        let name_start = LANG_LEN + 1;
        let name_end = name_start + name_len;
        let text_len = usize::from(
            *body
                .get(name_end)
                .ok_or(bad("name_length runs past descriptor end"))?,
        );
        let text_start = name_end + 1;
        let text_end = text_start + text_len;
        if text_end > body.len() {
            return Err(bad("text_length runs past descriptor end"));
        }
        Ok(Self {
            language_code: LangCode([body[0], body[1], body[2]]),
            name: DvbText::new(&body[name_start..name_end]),
            text: DvbText::new(&body[text_start..text_end]),
            trailing: &body[text_end..],
        })
    }
}

impl Serialize for SsuEventNameDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + LANG_LEN + 1 + self.name.len() + 1 + self.text.len() + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let name_len = broadcast_common::len::fit_u8(self.name.len(), "name_length")?;
        let text_len = broadcast_common::len::fit_u8(self.text.len(), "text_length")?;
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2..5].copy_from_slice(&self.language_code.0);
        buf[5] = name_len;
        let n_end = 6 + self.name.len();
        buf[6..n_end].copy_from_slice(self.name.raw());
        buf[n_end] = text_len;
        let t_start = n_end + 1;
        let t_end = t_start + self.text.len();
        buf[t_start..t_end].copy_from_slice(self.text.raw());
        buf[t_end..len].copy_from_slice(self.trailing);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for SsuEventNameDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SSU_EVENT_NAME";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let bytes = [
            TAG, 12, b'f', b'r', b'e', 3, b'S', b'S', b'U', 4, b'M', b'a', b'j', b'!',
        ];
        let d = SsuEventNameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.language_code, LangCode(*b"fre"));
        assert_eq!(d.name.raw(), b"SSU");
        assert_eq!(d.text.raw(), b"Maj!");
        assert!(d.trailing.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_strings_and_trailing_round_trip() {
        for bytes in [
            &[TAG, 5, b'e', b'n', b'g', 0, 0][..],
            &[TAG, 7, b'e', b'n', b'g', 0, 0, 0xAA, 0xBB][..],
        ] {
            let d = SsuEventNameDescriptor::parse(bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn rejects_overruns_and_wrong_tag() {
        assert!(SsuEventNameDescriptor::parse(&[TAG, 5, b'e', b'n', b'g', 100, 0]).is_err());
        assert!(SsuEventNameDescriptor::parse(&[TAG, 6, b'e', b'n', b'g', 1, b'x', 9]).is_err());
        assert!(SsuEventNameDescriptor::parse(&[TAG, 4, b'e', b'n', b'g', 0]).is_err());
        assert!(SsuEventNameDescriptor::parse(&[0x04, 5, b'e', b'n', b'g', 0, 0]).is_err());
    }
}
