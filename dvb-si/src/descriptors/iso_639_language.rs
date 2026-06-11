//! ISO 639 Language Descriptor — MPEG-2 ISO/IEC 13818-1 §2.6.19 (tag 0x0A).

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::LangCode;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for iso_639_language_descriptor.
pub const TAG: u8 = 0x0A;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 4;

/// One (language code, audio type) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LanguageEntry {
    /// Three-character ISO 639-2 language code (e.g. `LangCode(*b"eng")`).
    pub language_code: LangCode,
    /// Audio type (ETSI EN 300 468 §6.2.22): 0 = undefined, 1 = clean effects,
    /// 2 = hearing impaired, 3 = visual impaired commentary.
    pub audio_type: u8,
}

/// ISO 639 Language Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Iso639LanguageDescriptor {
    /// One or more language entries.
    pub entries: Vec<LanguageEntry>,
}

impl<'a> Parse<'a> for Iso639LanguageDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Iso639LanguageDescriptor",
            "unexpected tag for iso_639_language_descriptor",
        )?;
        if body.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "iso_639_language_descriptor length not a multiple of 4",
            });
        }
        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            entries.push(LanguageEntry {
                language_code: LangCode([chunk[0], chunk[1], chunk[2]]),
                audio_type: chunk[3],
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for Iso639LanguageDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.entries.len() * ENTRY_LEN
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
        buf[1] = (self.entries.len() * ENTRY_LEN) as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            buf[pos..pos + 3].copy_from_slice(&e.language_code.0);
            buf[pos + 3] = e.audio_type;
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for Iso639LanguageDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ISO_639_LANGUAGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_language_entry() {
        let bytes = [TAG, 4, b'e', b'n', b'g', 0x00];
        let d = Iso639LanguageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].language_code, LangCode(*b"eng"));
        assert_eq!(d.entries[0].audio_type, 0);
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = [TAG, 8, b'e', b'n', b'g', 1, b'f', b'r', b'a', 2];
        let d = Iso639LanguageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].language_code, LangCode(*b"fra"));
        assert_eq!(d.entries[1].audio_type, 2);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = Iso639LanguageDescriptor::parse(&[0x0B, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x0B, .. }));
    }

    #[test]
    fn parse_rejects_short_header() {
        let err = Iso639LanguageDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_4() {
        let bytes = [TAG, 5, b'e', b'n', b'g', 0, 0];
        let err = Iso639LanguageDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = Iso639LanguageDescriptor {
            entries: vec![
                LanguageEntry {
                    language_code: LangCode(*b"eng"),
                    audio_type: 0,
                },
                LanguageEntry {
                    language_code: LangCode(*b"fra"),
                    audio_type: 1,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = Iso639LanguageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = Iso639LanguageDescriptor {
            entries: vec![LanguageEntry {
                language_code: LangCode(*b"eng"),
                audio_type: 0,
            }],
        };
        assert_eq!(d.serialized_len() - 2, 4);
    }
}
