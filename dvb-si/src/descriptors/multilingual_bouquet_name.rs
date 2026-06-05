//! Multilingual Bouquet Name Descriptor — ETSI EN 300 468 §6.2.22 (tag 0x5C).
//!
//! Table 76 (PDF p. 93). Carried in the BAT. A loop of (ISO 639-2 language
//! code, bouquet name) pairs, each name length-prefixed by an 8-bit field.

use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for multilingual_bouquet_name_descriptor.
pub const TAG: u8 = 0x5C;
const HEADER_LEN: usize = 2;
const LANG_LEN: usize = 3;
const NAME_LEN_FIELD: usize = 1;

/// One localised bouquet name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BouquetNameEntry<'a> {
    /// ISO 639-2 language code.
    pub language_code: LangCode,
    /// DVB Annex-A encoded bouquet name.
    pub bouquet_name: DvbText<'a>,
}

/// Multilingual Bouquet Name Descriptor (tag 0x5C).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MultilingualBouquetNameDescriptor<'a> {
    /// Localised names in wire order.
    pub entries: Vec<BouquetNameEntry<'a>>,
}

impl<'a> Parse<'a> for MultilingualBouquetNameDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "MultilingualBouquetNameDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for multilingual_bouquet_name_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "MultilingualBouquetNameDescriptor body",
            });
        }
        let mut entries = Vec::new();
        let mut pos = HEADER_LEN;
        while pos < end {
            if pos + LANG_LEN + NAME_LEN_FIELD > end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "entry header runs past descriptor end",
                });
            }
            let language_code = LangCode([bytes[pos], bytes[pos + 1], bytes[pos + 2]]);
            let name_len = bytes[pos + LANG_LEN] as usize;
            let name_start = pos + LANG_LEN + NAME_LEN_FIELD;
            let name_end = name_start + name_len;
            if name_end > end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "name_length runs past descriptor end",
                });
            }
            entries.push(BouquetNameEntry {
                language_code,
                bouquet_name: DvbText::new(&bytes[name_start..name_end]),
            });
            pos = name_end;
        }
        Ok(Self { entries })
    }
}

impl Serialize for MultilingualBouquetNameDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self
                .entries
                .iter()
                .map(|e| LANG_LEN + NAME_LEN_FIELD + e.bouquet_name.len())
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        for e in &self.entries {
            if e.bouquet_name.len() > u8::MAX as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "bouquet_name exceeds 255 bytes (name_length is 8-bit)",
                });
            }
        }
        let len = self.serialized_len();
        let body = len - HEADER_LEN;
        if body > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "multilingual_bouquet_name_descriptor body exceeds 255 bytes",
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
        for e in &self.entries {
            buf[pos..pos + LANG_LEN].copy_from_slice(&e.language_code.0);
            buf[pos + LANG_LEN] = e.bouquet_name.len() as u8;
            let name_start = pos + LANG_LEN + NAME_LEN_FIELD;
            buf[name_start..name_start + e.bouquet_name.len()]
                .copy_from_slice(e.bouquet_name.raw());
            pos = name_start + e.bouquet_name.len();
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for MultilingualBouquetNameDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for MultilingualBouquetNameDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MULTILINGUAL_BOUQUET_NAME";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(entries: &[([u8; 3], &[u8])]) -> Vec<u8> {
        let body: usize = entries.iter().map(|(_, n)| LANG_LEN + 1 + n.len()).sum();
        let mut v = Vec::with_capacity(HEADER_LEN + body);
        v.push(TAG);
        v.push(body as u8);
        for (lang, name) in entries {
            v.extend_from_slice(lang);
            v.push(name.len() as u8);
            v.extend_from_slice(name);
        }
        v
    }

    #[test]
    fn parse_single_entry() {
        let bytes = build(&[(*b"eng", b"Sky")]);
        let d = MultilingualBouquetNameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].language_code, LangCode(*b"eng"));
        assert_eq!(d.entries[0].bouquet_name.raw(), b"Sky");
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = build(&[(*b"eng", b"Pack"), (*b"fra", b"Bouquet")]);
        let d = MultilingualBouquetNameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].bouquet_name.raw(), b"Bouquet");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MultilingualBouquetNameDescriptor::parse(&[0x5B, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x5B, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = MultilingualBouquetNameDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_name_length_overrun() {
        let bytes = [TAG, 5, b'e', b'n', b'g', 100, 0];
        let err = MultilingualBouquetNameDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_truncated_entry_header() {
        let bytes = [TAG, 2, b'e', b'n'];
        let err = MultilingualBouquetNameDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn empty_descriptor_valid() {
        let d = MultilingualBouquetNameDescriptor::parse(&[TAG, 0]).unwrap();
        assert_eq!(d.entries.len(), 0);
    }

    #[test]
    fn serialize_round_trip() {
        let bytes = build(&[(*b"eng", b"Bouquet"), (*b"deu", b"Paket")]);
        let parsed = MultilingualBouquetNameDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
        let re = MultilingualBouquetNameDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = MultilingualBouquetNameDescriptor {
            entries: vec![BouquetNameEntry {
                language_code: LangCode(*b"eng"),
                bouquet_name: DvbText::new(b"X"),
            }],
        };
        let mut tiny = [0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_range_name() {
        let name = vec![0u8; 256];
        let d = MultilingualBouquetNameDescriptor {
            entries: vec![BouquetNameEntry {
                language_code: LangCode(*b"eng"),
                bouquet_name: DvbText::new(&name),
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
        let d = MultilingualBouquetNameDescriptor {
            entries: vec![BouquetNameEntry {
                language_code: LangCode(*b"eng"),
                bouquet_name: DvbText::new(b"Sky"),
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, serde_json::to_string(&d.clone()).unwrap());
    }
}
