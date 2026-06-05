//! Extended Event Descriptor — ETSI EN 300 468 §6.2.16 (tag 0x4E).
//!
//! Carried inside EIT. Provides the full event description split across
//! descriptor_number / last_descriptor_number fragments. Each fragment
//! contributes text plus an optional list of (item_description, item) pairs.

use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for extended_event_descriptor.
pub const TAG: u8 = 0x4E;
const HEADER_LEN: usize = 2;
const NUMBERS_LEN: usize = 1;
const LANG_LEN: usize = 3;
const ITEMS_LEN_FIELD: usize = 1;
const TEXT_LEN_FIELD: usize = 1;

/// One (description, value) item — e.g. "Director" → "Alice Smith".
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ExtendedEventItem<'a> {
    /// DVB Annex-A encoded item description.
    pub description: DvbText<'a>,
    /// DVB Annex-A encoded item value.
    pub value: DvbText<'a>,
}

/// Extended Event Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ExtendedEventDescriptor<'a> {
    /// 0-based fragment index within the extended event series.
    pub descriptor_number: u8,
    /// Index of the final fragment in the series (0-based).
    pub last_descriptor_number: u8,
    /// ISO 639-2 language code.
    pub language_code: LangCode,
    /// Item list.
    pub items: Vec<ExtendedEventItem<'a>>,
    /// DVB Annex-A encoded extended text.
    pub text: DvbText<'a>,
}

impl<'a> Parse<'a> for ExtendedEventDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + NUMBERS_LEN + LANG_LEN + ITEMS_LEN_FIELD + TEXT_LEN_FIELD;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "ExtendedEventDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for extended_event_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "ExtendedEventDescriptor body",
            });
        }
        let numbers_byte = bytes[2];
        let descriptor_number = numbers_byte >> 4;
        let last_descriptor_number = numbers_byte & 0x0F;
        let language_code = LangCode([bytes[3], bytes[4], bytes[5]]);

        let items_len_pos = HEADER_LEN + NUMBERS_LEN + LANG_LEN;
        let items_length = bytes[items_len_pos] as usize;
        let items_start = items_len_pos + ITEMS_LEN_FIELD;
        let items_end = items_start + items_length;
        if items_end + TEXT_LEN_FIELD > end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "length_of_items runs past descriptor end",
            });
        }

        let mut items = Vec::new();
        let mut pos = items_start;
        while pos < items_end {
            if pos >= items_end {
                break;
            }
            let desc_len = bytes[pos] as usize;
            let desc_start = pos + 1;
            let desc_end = desc_start + desc_len;
            if desc_end + 1 > items_end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "item_description_length runs past items loop",
                });
            }
            let value_len = bytes[desc_end] as usize;
            let value_start = desc_end + 1;
            let value_end = value_start + value_len;
            if value_end > items_end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "item_length runs past items loop",
                });
            }
            items.push(ExtendedEventItem {
                description: DvbText::new(&bytes[desc_start..desc_end]),
                value: DvbText::new(&bytes[value_start..value_end]),
            });
            pos = value_end;
        }

        let text_len = bytes[items_end] as usize;
        let text_start = items_end + TEXT_LEN_FIELD;
        let text_end = text_start + text_len;
        if text_end > end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "text_length runs past descriptor end",
            });
        }
        let text = DvbText::new(&bytes[text_start..text_end]);

        Ok(Self {
            descriptor_number,
            last_descriptor_number,
            language_code,
            items,
            text,
        })
    }
}

impl Serialize for ExtendedEventDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let items_bytes: usize = self
            .items
            .iter()
            .map(|i| 1 + i.description.len() + 1 + i.value.len())
            .sum();
        HEADER_LEN
            + NUMBERS_LEN
            + LANG_LEN
            + ITEMS_LEN_FIELD
            + items_bytes
            + TEXT_LEN_FIELD
            + self.text.len()
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
        buf[2] = ((self.descriptor_number & 0x0F) << 4) | (self.last_descriptor_number & 0x0F);
        buf[3..6].copy_from_slice(&self.language_code.0);

        let items_bytes: usize = self
            .items
            .iter()
            .map(|i| 1 + i.description.len() + 1 + i.value.len())
            .sum();
        buf[6] = items_bytes as u8;

        let mut pos = 7;
        for item in &self.items {
            buf[pos] = item.description.len() as u8;
            let d_start = pos + 1;
            let d_end = d_start + item.description.len();
            buf[d_start..d_end].copy_from_slice(item.description.raw());
            buf[d_end] = item.value.len() as u8;
            let v_start = d_end + 1;
            let v_end = v_start + item.value.len();
            buf[v_start..v_end].copy_from_slice(item.value.raw());
            pos = v_end;
        }
        buf[pos] = self.text.len() as u8;
        buf[pos + 1..pos + 1 + self.text.len()].copy_from_slice(self.text.raw());
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for ExtendedEventDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ExtendedEventDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "EXTENDED_EVENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(
        descriptor_number: u8,
        last_descriptor_number: u8,
        lang: [u8; 3],
        items: &[(&[u8], &[u8])],
        text: &[u8],
    ) -> Vec<u8> {
        let items_bytes: Vec<u8> = items
            .iter()
            .flat_map(|(d, v)| {
                let mut x = Vec::with_capacity(2 + d.len() + v.len());
                x.push(d.len() as u8);
                x.extend_from_slice(d);
                x.push(v.len() as u8);
                x.extend_from_slice(v);
                x
            })
            .collect();
        let body_len = NUMBERS_LEN
            + LANG_LEN
            + ITEMS_LEN_FIELD
            + items_bytes.len()
            + TEXT_LEN_FIELD
            + text.len();
        let mut v = Vec::with_capacity(HEADER_LEN + body_len);
        v.push(TAG);
        v.push(body_len as u8);
        v.push(((descriptor_number & 0x0F) << 4) | (last_descriptor_number & 0x0F));
        v.extend_from_slice(&lang);
        v.push(items_bytes.len() as u8);
        v.extend_from_slice(&items_bytes);
        v.push(text.len() as u8);
        v.extend_from_slice(text);
        v
    }

    #[test]
    fn parse_extracts_numbers_language_and_text() {
        let bytes = build(2, 3, *b"fra", &[], b"Hello");
        let d = ExtendedEventDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.descriptor_number, 2);
        assert_eq!(d.last_descriptor_number, 3);
        assert_eq!(d.language_code, LangCode(*b"fra"));
        assert_eq!(d.text.raw(), b"Hello");
        assert_eq!(d.items.len(), 0);
    }

    #[test]
    fn parse_extracts_item_list() {
        let items: [(&[u8], &[u8]); 2] = [(b"Director", b"Alice"), (b"Year", b"2023")];
        let bytes = build(0, 0, *b"eng", &items, b"");
        let d = ExtendedEventDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.items.len(), 2);
        assert_eq!(d.items[0].description.raw(), b"Director");
        assert_eq!(d.items[0].value.raw(), b"Alice");
        assert_eq!(d.items[1].description.raw(), b"Year");
        assert_eq!(d.items[1].value.raw(), b"2023");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        // Full 8-byte descriptor with the wrong tag — must fail with tag mismatch.
        let bytes = [0x4D, 6, 0x00, b'e', b'n', b'g', 0, 0];
        assert!(matches!(
            ExtendedEventDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x4D, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_header() {
        assert!(matches!(
            ExtendedEventDescriptor::parse(&[TAG, 2, 0, 0]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_item_length_overrun() {
        // items_length=10 but body only 6 bytes after the items_length field.
        let bytes = [
            TAG, 11, 0x00, b'e', b'n', b'g', 10, // items_length claims 10 bytes
            0, 0, 0, 0, 0, // only 5 bytes of items
            0, // text_length (partially unreachable)
        ];
        assert!(matches!(
            ExtendedEventDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn parse_rejects_text_length_overrun() {
        // body=6: numbers + lang + items_length=0 + text_length=5 but no text bytes.
        let bytes = [TAG, 6, 0x00, b'e', b'n', b'g', 0, 5];
        assert!(matches!(
            ExtendedEventDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn empty_items_and_text_valid() {
        let bytes = build(0, 0, *b"eng", &[], b"");
        let d = ExtendedEventDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.items.len(), 0);
        assert_eq!(d.text.raw(), b"");
    }

    #[test]
    fn serialize_round_trip_with_items_and_text() {
        let items: [(&[u8], &[u8]); 1] = [(b"Genre", b"Drama")];
        let bytes = build(1, 2, *b"eng", &items, b"Plot summary.");
        let parsed = ExtendedEventDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
        let re = ExtendedEventDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, re);
    }

    #[test]
    fn serialize_round_trip_without_items() {
        let bytes = build(3, 3, *b"fra", &[], b"Plot.");
        let parsed = ExtendedEventDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }
}
