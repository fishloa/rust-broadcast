//! Application Name Descriptor — ETSI TS 102 809 §5.3.5.6.2, Table 24
//! (AIT tag 0x01).
//!
//! Carried in the AIT per-application descriptor loop. A multilingual loop of
//! (ISO 639 language code + name) pairs, following the same pattern as the
//! SI multilingual descriptors.

use crate::descriptors::descriptor_body;
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for application_name_descriptor (AIT namespace).
pub const TAG: u8 = 0x01;
const HEADER_LEN: usize = 2;
const LANG_LEN: usize = 3;
const NAME_LEN_FIELD: usize = 1;

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
        let mut pos = 0;
        while pos < body.len() {
            if pos + LANG_LEN + NAME_LEN_FIELD > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "entry header runs past descriptor end",
                });
            }
            let language_code = LangCode([body[pos], body[pos + 1], body[pos + 2]]);
            let name_len = body[pos + LANG_LEN] as usize;
            let name_start = pos + LANG_LEN + NAME_LEN_FIELD;
            let name_end = name_start + name_len;
            if name_end > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "application_name_length runs past descriptor end",
                });
            }
            entries.push(ApplicationNameEntry {
                language_code,
                application_name: DvbText::new(&body[name_start..name_end]),
            });
            pos = name_end;
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
                .map(|e| LANG_LEN + NAME_LEN_FIELD + e.application_name.len())
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        for e in &self.entries {
            if e.application_name.len() > u8::MAX as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "application_name exceeds 255 bytes",
                });
            }
        }
        let len = self.serialized_len();
        let body_len = len - HEADER_LEN;
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_name_descriptor body exceeds 255 bytes",
            });
        }
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            buf[pos..pos + LANG_LEN].copy_from_slice(&e.language_code.0);
            buf[pos + LANG_LEN] = e.application_name.len() as u8;
            let name_start = pos + LANG_LEN + NAME_LEN_FIELD;
            buf[name_start..name_start + e.application_name.len()]
                .copy_from_slice(e.application_name.raw());
            pos = name_start + e.application_name.len();
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
