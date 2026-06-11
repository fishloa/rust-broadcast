//! Data Broadcast Descriptor — ETSI EN 300 468 §6.2.12 (tag 0x64).
//!
//! Table 31 (PDF p. 71). Identifies a data broadcast component: its
//! data_broadcast_id, the component_tag tying it to a stream_identifier, a raw
//! selector tail, plus a localised text description in one language.

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for data_broadcast_descriptor.
pub const TAG: u8 = 0x64;
const HEADER_LEN: usize = 2;
const ID_LEN: usize = 2;
const COMPONENT_TAG_LEN: usize = 1;
const SELECTOR_LEN_FIELD: usize = 1;
const LANG_LEN: usize = 3;
const TEXT_LEN_FIELD: usize = 1;

/// Data Broadcast Descriptor (tag 0x64).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DataBroadcastDescriptor<'a> {
    /// 16-bit data_broadcast_id (ETSI TS 101 162 registration).
    pub data_broadcast_id: u16,
    /// component_tag linking this entry to a stream_identifier_descriptor.
    pub component_tag: u8,
    /// Raw selector_byte tail — interpretation depends on data_broadcast_id.
    /// Kept raw deliberately; decode via id-specific parsers per TS 101 162.
    pub selector: &'a [u8],
    /// ISO 639-2 language code of the text description.
    pub language_code: LangCode,
    /// DVB Annex-A encoded text description.
    pub text: DvbText<'a>,
}

impl<'a> Parse<'a> for DataBroadcastDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_body = ID_LEN + COMPONENT_TAG_LEN + SELECTOR_LEN_FIELD + LANG_LEN + TEXT_LEN_FIELD;
        let body = descriptor_body(
            bytes,
            TAG,
            "DataBroadcastDescriptor",
            "unexpected tag for data_broadcast_descriptor",
        )?;
        if body.len() < min_body {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "data_broadcast_descriptor body shorter than minimum 8 bytes",
            });
        }
        let mut pos = 0;
        let data_broadcast_id = u16::from_be_bytes([body[pos], body[pos + 1]]);
        pos += ID_LEN;
        let component_tag = body[pos];
        pos += COMPONENT_TAG_LEN;

        let selector_length = body[pos] as usize;
        pos += SELECTOR_LEN_FIELD;
        let selector_end = pos + selector_length;
        // Need selector + lang(3) + text_length(1) to still fit.
        if selector_end + LANG_LEN + TEXT_LEN_FIELD > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "selector_length runs past descriptor end",
            });
        }
        let selector = &body[pos..selector_end];
        pos = selector_end;

        let language_code = LangCode([body[pos], body[pos + 1], body[pos + 2]]);
        pos += LANG_LEN;

        let text_length = body[pos] as usize;
        pos += TEXT_LEN_FIELD;
        let text_end = pos + text_length;
        if text_end > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "text_length runs past descriptor end",
            });
        }
        let text = DvbText::new(&body[pos..text_end]);

        Ok(Self {
            data_broadcast_id,
            component_tag,
            selector,
            language_code,
            text,
        })
    }
}

impl Serialize for DataBroadcastDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + ID_LEN
            + COMPONENT_TAG_LEN
            + SELECTOR_LEN_FIELD
            + self.selector.len()
            + LANG_LEN
            + TEXT_LEN_FIELD
            + self.text.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.selector.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "selector exceeds 255 bytes (selector_length is 8-bit)",
            });
        }
        if self.text.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "text exceeds 255 bytes (text_length is 8-bit)",
            });
        }
        let len = self.serialized_len();
        let body = len - HEADER_LEN;
        if body > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "data_broadcast_descriptor body exceeds 255 bytes",
            });
        }
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body as u8;
        let mut pos = HEADER_LEN;
        buf[pos..pos + ID_LEN].copy_from_slice(&self.data_broadcast_id.to_be_bytes());
        pos += ID_LEN;
        buf[pos] = self.component_tag;
        pos += COMPONENT_TAG_LEN;
        buf[pos] = self.selector.len() as u8;
        pos += SELECTOR_LEN_FIELD;
        buf[pos..pos + self.selector.len()].copy_from_slice(self.selector);
        pos += self.selector.len();
        buf[pos..pos + LANG_LEN].copy_from_slice(&self.language_code.0);
        pos += LANG_LEN;
        buf[pos] = self.text.len() as u8;
        pos += TEXT_LEN_FIELD;
        buf[pos..pos + self.text.len()].copy_from_slice(self.text.raw());
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for DataBroadcastDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DATA_BROADCAST";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(id: u16, ctag: u8, selector: &[u8], lang: [u8; 3], text: &[u8]) -> Vec<u8> {
        let body = ID_LEN
            + COMPONENT_TAG_LEN
            + SELECTOR_LEN_FIELD
            + selector.len()
            + LANG_LEN
            + TEXT_LEN_FIELD
            + text.len();
        let mut v = Vec::with_capacity(HEADER_LEN + body);
        v.push(TAG);
        v.push(body as u8);
        v.extend_from_slice(&id.to_be_bytes());
        v.push(ctag);
        v.push(selector.len() as u8);
        v.extend_from_slice(selector);
        v.extend_from_slice(&lang);
        v.push(text.len() as u8);
        v.extend_from_slice(text);
        v
    }

    #[test]
    fn parse_extracts_all_fields() {
        let bytes = build(0x000B, 0x12, &[0xAA, 0xBB], *b"eng", b"Hello");
        let d = DataBroadcastDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.data_broadcast_id, 0x000B);
        assert_eq!(d.component_tag, 0x12);
        assert_eq!(d.selector, &[0xAA, 0xBB]);
        assert_eq!(d.language_code, LangCode(*b"eng"));
        assert_eq!(d.text.raw(), b"Hello");
    }

    #[test]
    fn parse_accepts_empty_selector_and_text() {
        let bytes = build(0x0001, 0x00, &[], *b"fra", b"");
        let d = DataBroadcastDescriptor::parse(&bytes).unwrap();
        assert!(d.selector.is_empty());
        assert!(d.text.raw().is_empty());
        assert_eq!(d.language_code, LangCode(*b"fra"));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build(0x0001, 0x00, &[], *b"eng", b"");
        bytes[0] = 0x65;
        let err = DataBroadcastDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x65, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = DataBroadcastDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_body_too_short() {
        // length=4: cannot hold the 8-byte minimum.
        let err = DataBroadcastDescriptor::parse(&[TAG, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_selector_length_overrun() {
        // selector_length=200 but body is tiny.
        let bytes = [TAG, 8, 0x00, 0x0B, 0x12, 200, b'e', b'n', b'g', 0];
        let err = DataBroadcastDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_text_length_overrun() {
        // selector_length=0, lang present, text_length=5 but no text bytes.
        let bytes = [TAG, 8, 0x00, 0x0B, 0x12, 0, b'e', b'n', b'g', 5];
        let err = DataBroadcastDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let bytes = build(0x0123, 0x45, &[0xDE, 0xAD], *b"deu", b"Daten");
        let parsed = DataBroadcastDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
        let re = DataBroadcastDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = DataBroadcastDescriptor {
            data_broadcast_id: 0x0001,
            component_tag: 0x00,
            selector: &[],
            language_code: LangCode(*b"eng"),
            text: DvbText::new(&[]),
        };
        let mut tiny = [0u8; 4];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_range_selector() {
        let sel = vec![0u8; 256];
        let d = DataBroadcastDescriptor {
            data_broadcast_id: 0x0001,
            component_tag: 0x00,
            selector: &sel,
            language_code: LangCode(*b"eng"),
            text: DvbText::new(&[]),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        // selector 250 + text 10 + fixed 7 = 267 > 255, both sub-fields in range.
        let sel = vec![0u8; 250];
        let txt = vec![0u8; 10];
        let d = DataBroadcastDescriptor {
            data_broadcast_id: 0x0001,
            component_tag: 0x00,
            selector: &sel,
            language_code: LangCode(*b"eng"),
            text: DvbText::new(&txt),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_is_stable() {
        let d = DataBroadcastDescriptor {
            data_broadcast_id: 0x000B,
            component_tag: 0x09,
            selector: &[0x01, 0x02],
            language_code: LangCode(*b"eng"),
            text: DvbText::new(b"Text"),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"data_broadcast_id\""));
        assert!(json.contains("\"component_tag\""));
        assert!(json.contains("\"eng\""));
        assert!(json.contains("\"Text\""));
    }
}
