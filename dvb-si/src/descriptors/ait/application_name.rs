//! Application Name Descriptor — ETSI TS 102 809 §5.3.5.6.2, Table 24
//! (AIT tag 0x01).
//!
//! Carried in the AIT per-application descriptor loop. A multilingual loop of
//! (ISO 639 language code + name) pairs, following the same pattern as the
//! SI multilingual descriptors.

use crate::descriptors::descriptor_body;
use crate::descriptors::lang_text::{
    EntryReader, LANG_LEN, text_field_len, write_lang, write_text,
};
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for application_name_descriptor (AIT namespace).
pub const TAG: u8 = 0x01;
const HEADER_LEN: usize = 2;

/// One localised application name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ApplicationNameEntry<'a> {
    /// ISO 639-2 language code.
    pub language_code: LangCode,
    /// DVB Annex-A encoded application name.
    pub application_name: DvbText<'a>,
}

/// Application Name Descriptor (AIT tag 0x01).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ApplicationNameDescriptor<'a> {
    /// Localised names in wire order.
    pub entries: Vec<ApplicationNameEntry<'a>>,
}

impl<'a> Parse<'a> for ApplicationNameDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ApplicationNameDescriptor",
            "unexpected tag for application_name_descriptor",
        )?;
        let mut entries = Vec::new();
        let mut reader = EntryReader::new(body, 0, TAG);
        while reader.has_more() {
            let language_code = reader.lang()?;
            let application_name =
                reader.text("application_name_length runs past descriptor end", 0)?;
            entries.push(ApplicationNameEntry {
                language_code,
                application_name,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for ApplicationNameDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self
                .entries
                .iter()
                .map(|e| LANG_LEN + text_field_len(&e.application_name))
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let body_len = len - HEADER_LEN;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, body_len)?;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            pos = write_lang(buf, pos, &e.language_code);
            pos = write_text(buf, pos, &e.application_name, "application_name_length")?;
        }
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ApplicationNameDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "APPLICATION_NAME";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift fix (#1141): the over-range name check is the shared
    /// `lang_text::write_text` one, exercised here through this former copy.
    #[test]
    fn serialize_rejects_over_range_name() {
        let name = alloc::vec![0u8; 256];
        let d = ApplicationNameDescriptor {
            entries: alloc::vec![ApplicationNameEntry {
                language_code: LangCode(*b"eng"),
                application_name: DvbText::new(&name),
            }],
        };
        let mut buf = alloc::vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    /// Body: lang(3) + name_len(1) + "Foo"(3) = 7.
    fn build_single_entry_foo() -> [u8; 9] {
        [TAG, 7, b'e', b'n', b'g', 3, b'F', b'o', b'o']
    }

    /// Body: "eng"/"Foo"(7) + "fra"/"Bar"(7) = 14.
    fn build_two_entries() -> [u8; 16] {
        [
            TAG, 14, b'e', b'n', b'g', 3, b'F', b'o', b'o', b'f', b'r', b'a', 3, b'B', b'a', b'r',
        ]
    }

    #[test]
    fn parse_single_entry() {
        let bytes = build_single_entry_foo();
        let d = ApplicationNameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].language_code, LangCode(*b"eng"));
        assert_eq!(d.entries[0].application_name.raw(), b"Foo");
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = build_two_entries();
        let d = ApplicationNameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].language_code, LangCode(*b"eng"));
        assert_eq!(d.entries[1].language_code, LangCode(*b"fra"));
    }

    #[test]
    fn serialize_round_trip() {
        let d = ApplicationNameDescriptor {
            entries: alloc::vec![
                ApplicationNameEntry {
                    language_code: LangCode(*b"eng"),
                    application_name: DvbText::new(b"HbbTV"),
                },
                ApplicationNameEntry {
                    language_code: LangCode(*b"deu"),
                    application_name: DvbText::new(b"App"),
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ApplicationNameDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_byte_identical_single() {
        let bytes = build_single_entry_foo();
        let d = ApplicationNameDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }

    #[test]
    fn serialize_byte_identical_two() {
        let bytes = build_two_entries();
        let d = ApplicationNameDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }
}
