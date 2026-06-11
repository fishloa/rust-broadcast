//! Short Event Descriptor — ETSI EN 300 468 §6.2.37 (tag 0x4D).
//!
//! Carried inside EIT. Gives the event's title and brief description in a
//! single language.

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for short_event_descriptor.
pub const TAG: u8 = 0x4D;
const HEADER_LEN: usize = 2;
const LANG_LEN: usize = 3;

/// Short Event Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ShortEventDescriptor<'a> {
    /// ISO 639-2 language code of the event name / text.
    pub language_code: LangCode,
    /// DVB Annex-A encoded event name.
    pub event_name: DvbText<'a>,
    /// DVB Annex-A encoded brief description.
    pub text: DvbText<'a>,
}

impl<'a> Parse<'a> for ShortEventDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ShortEventDescriptor",
            "unexpected tag for short_event_descriptor",
        )?;
        if body.len() < LANG_LEN + 2 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "short_event_descriptor body shorter than minimum 5 bytes",
            });
        }
        let language_code = LangCode([body[0], body[1], body[2]]);
        let name_len = body[LANG_LEN] as usize;
        let name_start = LANG_LEN + 1;
        let name_end = name_start + name_len;
        if name_end + 1 > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "event_name_length runs past descriptor end",
            });
        }
        let event_name = DvbText::new(&body[name_start..name_end]);
        let text_len = body[name_end] as usize;
        let text_start = name_end + 1;
        let text_end = text_start + text_len;
        if text_end > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "text_length runs past descriptor end",
            });
        }
        let text = DvbText::new(&body[text_start..text_end]);
        Ok(Self {
            language_code,
            event_name,
            text,
        })
    }
}

impl Serialize for ShortEventDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + LANG_LEN + 1 + self.event_name.len() + 1 + self.text.len()
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
        buf[2..5].copy_from_slice(&self.language_code.0);
        buf[5] = self.event_name.len() as u8;
        let n_start = 6;
        let n_end = n_start + self.event_name.len();
        buf[n_start..n_end].copy_from_slice(self.event_name.raw());
        buf[n_end] = self.text.len() as u8;
        let t_start = n_end + 1;
        buf[t_start..t_start + self.text.len()].copy_from_slice(self.text.raw());
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for ShortEventDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SHORT_EVENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_all_fields() {
        let bytes = [
            TAG, 0x0C, b'e', b'n', b'g', 4, b'N', b'e', b'w', b's', 3, b'L', b'i', b'v',
        ];
        let d = ShortEventDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.language_code, LangCode(*b"eng"));
        assert_eq!(d.event_name.raw(), b"News");
        assert_eq!(d.text.raw(), b"Liv");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = ShortEventDescriptor::parse(&[0x4E, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x4E, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = ShortEventDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_body_too_short_for_minimum() {
        // Body of length 3 has language code but no event_name length field.
        let err = ShortEventDescriptor::parse(&[TAG, 3, b'e', b'n', b'g']).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_name_length_overrun() {
        // body length 5 = lang(3) + name_len(1) + 1 byte of name/text.
        // Set name_len = 100 — bigger than the remaining body.
        let bytes = [TAG, 5, b'e', b'n', b'g', 100, 0];
        let err = ShortEventDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn empty_event_name_and_text_valid() {
        let bytes = [TAG, 5, b'e', b'n', b'g', 0, 0];
        let d = ShortEventDescriptor::parse(&bytes).unwrap();
        assert!(d.event_name.raw().is_empty());
        assert!(d.text.raw().is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = ShortEventDescriptor {
            language_code: LangCode(*b"fra"),
            event_name: DvbText::new(b"Journal"),
            text: DvbText::new(b"20h"),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ShortEventDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = ShortEventDescriptor {
            language_code: LangCode(*b"eng"),
            event_name: DvbText::new(b"ABC"),
            text: DvbText::new(b"DE"),
        };
        // 3 lang + 1 name_len + 3 name + 1 text_len + 2 text = 10
        assert_eq!(d.serialized_len() - 2, 10);
    }
}
