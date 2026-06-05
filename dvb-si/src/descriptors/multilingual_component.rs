//! Multilingual Component Descriptor — ETSI EN 300 468 §6.2.23 (tag 0x5E).
//!
//! Table 77 (PDF p. 94). Carried in the EIT / PMT. A leading `component_tag`
//! byte ties the descriptor to a component, followed by a loop of (ISO 639-2
//! language code, text) pairs, each text length-prefixed by an 8-bit field.

use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for multilingual_component_descriptor.
pub const TAG: u8 = 0x5E;
const HEADER_LEN: usize = 2;
const COMPONENT_TAG_LEN: usize = 1;
const LANG_LEN: usize = 3;
const TEXT_LEN_FIELD: usize = 1;

/// One localised component description.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ComponentTextEntry<'a> {
    /// ISO 639-2 language code.
    pub language_code: LangCode,
    /// DVB Annex-A encoded component description text.
    pub text: DvbText<'a>,
}

/// Multilingual Component Descriptor (tag 0x5E).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct MultilingualComponentDescriptor<'a> {
    /// component_tag linking this descriptor to a stream_identifier_descriptor.
    pub component_tag: u8,
    /// Localised descriptions in wire order.
    pub entries: Vec<ComponentTextEntry<'a>>,
}

impl<'a> Parse<'a> for MultilingualComponentDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "MultilingualComponentDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for multilingual_component_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "MultilingualComponentDescriptor body",
            });
        }
        if length < COMPONENT_TAG_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "multilingual_component_descriptor body missing component_tag",
            });
        }
        let component_tag = bytes[HEADER_LEN];
        let mut entries = Vec::new();
        let mut pos = HEADER_LEN + COMPONENT_TAG_LEN;
        while pos < end {
            if pos + LANG_LEN + TEXT_LEN_FIELD > end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "entry header runs past descriptor end",
                });
            }
            let language_code = LangCode([bytes[pos], bytes[pos + 1], bytes[pos + 2]]);
            let text_len = bytes[pos + LANG_LEN] as usize;
            let text_start = pos + LANG_LEN + TEXT_LEN_FIELD;
            let text_end = text_start + text_len;
            if text_end > end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "text_length runs past descriptor end",
                });
            }
            entries.push(ComponentTextEntry {
                language_code,
                text: DvbText::new(&bytes[text_start..text_end]),
            });
            pos = text_end;
        }
        Ok(Self {
            component_tag,
            entries,
        })
    }
}

impl Serialize for MultilingualComponentDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + COMPONENT_TAG_LEN
            + self
                .entries
                .iter()
                .map(|e| LANG_LEN + TEXT_LEN_FIELD + e.text.len())
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        for e in &self.entries {
            if e.text.len() > u8::MAX as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "text exceeds 255 bytes (text_length is 8-bit)",
                });
            }
        }
        let len = self.serialized_len();
        let body = len - HEADER_LEN;
        if body > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "multilingual_component_descriptor body exceeds 255 bytes",
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
        buf[HEADER_LEN] = self.component_tag;
        let mut pos = HEADER_LEN + COMPONENT_TAG_LEN;
        for e in &self.entries {
            buf[pos..pos + LANG_LEN].copy_from_slice(&e.language_code.0);
            buf[pos + LANG_LEN] = e.text.len() as u8;
            let text_start = pos + LANG_LEN + TEXT_LEN_FIELD;
            buf[text_start..text_start + e.text.len()].copy_from_slice(e.text.raw());
            pos = text_start + e.text.len();
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for MultilingualComponentDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for MultilingualComponentDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MULTILINGUAL_COMPONENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(component_tag: u8, entries: &[([u8; 3], &[u8])]) -> Vec<u8> {
        let body: usize = COMPONENT_TAG_LEN
            + entries
                .iter()
                .map(|(_, t)| LANG_LEN + 1 + t.len())
                .sum::<usize>();
        let mut v = Vec::with_capacity(HEADER_LEN + body);
        v.push(TAG);
        v.push(body as u8);
        v.push(component_tag);
        for (lang, text) in entries {
            v.extend_from_slice(lang);
            v.push(text.len() as u8);
            v.extend_from_slice(text);
        }
        v
    }

    #[test]
    fn parse_extracts_component_tag_and_entries() {
        let bytes = build(0x12, &[(*b"eng", b"Video")]);
        let d = MultilingualComponentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_tag, 0x12);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].language_code, LangCode(*b"eng"));
        assert_eq!(d.entries[0].text.raw(), b"Video");
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = build(0x03, &[(*b"eng", b"Audio"), (*b"fra", b"Son")]);
        let d = MultilingualComponentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_tag, 0x03);
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].text.raw(), b"Son");
    }

    #[test]
    fn parse_component_tag_only_valid() {
        // Body = just the component_tag, no language entries.
        let bytes = [TAG, 1, 0x09];
        let d = MultilingualComponentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_tag, 0x09);
        assert_eq!(d.entries.len(), 0);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MultilingualComponentDescriptor::parse(&[0x5D, 1, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x5D, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = MultilingualComponentDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_missing_component_tag() {
        // length=0: no component_tag.
        let err = MultilingualComponentDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_text_length_overrun() {
        // component_tag + lang + text_len=100 but no text bytes.
        let bytes = [TAG, 5, 0x01, b'e', b'n', b'g', 100];
        let err = MultilingualComponentDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let bytes = build(0x07, &[(*b"eng", b"Subtitle"), (*b"deu", b"Untertitel")]);
        let parsed = MultilingualComponentDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
        let re = MultilingualComponentDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = MultilingualComponentDescriptor {
            component_tag: 0x01,
            entries: vec![],
        };
        let mut tiny = [0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_range_text() {
        let text = vec![0u8; 256];
        let d = MultilingualComponentDescriptor {
            component_tag: 0x01,
            entries: vec![ComponentTextEntry {
                language_code: LangCode(*b"eng"),
                text: DvbText::new(&text),
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_is_stable() {
        // Borrowed `&[u8]` cannot be deserialized from a JSON array by
        // serde_json; matching the borrowed-bytes descriptors in this crate we
        // exercise the serialize path and assert it is deterministic.
        let d = MultilingualComponentDescriptor {
            component_tag: 0x12,
            entries: vec![ComponentTextEntry {
                language_code: LangCode(*b"eng"),
                text: DvbText::new(b"Video"),
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, serde_json::to_string(&d.clone()).unwrap());
    }
}
