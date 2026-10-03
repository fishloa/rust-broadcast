//! Multilingual Service Name Descriptor — ETSI EN 300 468 §6.2.25 (tag 0x5D).
//!
//! Table 79 (PDF p. 95). Carried in the SDT. A loop of entries, each carrying
//! an ISO 639-2 language code plus TWO length-prefixed strings: the service
//! provider name and the service name.

use super::descriptor_body;
use super::lang_text::{EntryReader, LANG_LEN, text_field_len, write_lang, write_text};
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for multilingual_service_name_descriptor.
pub const TAG: u8 = 0x5D;
const HEADER_LEN: usize = 2;

/// One localised (provider name, service name) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ServiceNameEntry<'a> {
    /// ISO 639-2 language code.
    pub language_code: LangCode,
    /// DVB Annex-A encoded service provider name.
    pub service_provider_name: DvbText<'a>,
    /// DVB Annex-A encoded service name.
    pub service_name: DvbText<'a>,
}

/// Multilingual Service Name Descriptor (tag 0x5D).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct MultilingualServiceNameDescriptor<'a> {
    /// Localised name pairs in wire order.
    pub entries: Vec<ServiceNameEntry<'a>>,
}

impl<'a> Parse<'a> for MultilingualServiceNameDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MultilingualServiceNameDescriptor",
            "unexpected tag for multilingual_service_name_descriptor",
        )?;
        let mut entries = Vec::new();
        let mut reader = EntryReader::new(body, 0, TAG);
        while reader.has_more() {
            let language_code = reader.lang()?;
            // One byte must follow the provider name: the service_name_length.
            let service_provider_name =
                reader.text("service_provider_name_length runs past descriptor end", 1)?;
            let service_name = reader.text("service_name_length runs past descriptor end", 0)?;
            entries.push(ServiceNameEntry {
                language_code,
                service_provider_name,
                service_name,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for MultilingualServiceNameDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self
                .entries
                .iter()
                .map(|e| {
                    LANG_LEN
                        + text_field_len(&e.service_provider_name)
                        + text_field_len(&e.service_name)
                })
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let body = len - HEADER_LEN;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, body)?;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            pos = write_lang(buf, pos, &e.language_code);
            pos = write_text(
                buf,
                pos,
                &e.service_provider_name,
                "service_provider_name_length",
            )?;
            pos = write_text(buf, pos, &e.service_name, "service_name_length")?;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for MultilingualServiceNameDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MULTILINGUAL_SERVICE_NAME";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(entries: &[([u8; 3], &[u8], &[u8])]) -> Vec<u8> {
        let body: usize = entries
            .iter()
            .map(|(_, p, s)| LANG_LEN + 1 + p.len() + 1 + s.len())
            .sum();
        let mut v = Vec::with_capacity(HEADER_LEN + body);
        v.push(TAG);
        v.push(body as u8);
        for (lang, provider, service) in entries {
            v.extend_from_slice(lang);
            v.push(provider.len() as u8);
            v.extend_from_slice(provider);
            v.push(service.len() as u8);
            v.extend_from_slice(service);
        }
        v
    }

    #[test]
    fn parse_single_entry() {
        let bytes = build(&[(*b"eng", b"BBC", b"One")]);
        let d = MultilingualServiceNameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].language_code, LangCode(*b"eng"));
        assert_eq!(d.entries[0].service_provider_name.raw(), b"BBC");
        assert_eq!(d.entries[0].service_name.raw(), b"One");
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = build(&[(*b"eng", b"Prov", b"Svc"), (*b"fra", b"Fourn", b"Chaine")]);
        let d = MultilingualServiceNameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].service_name.raw(), b"Chaine");
    }

    #[test]
    fn parse_empty_names_valid() {
        let bytes = build(&[(*b"deu", b"", b"")]);
        let d = MultilingualServiceNameDescriptor::parse(&bytes).unwrap();
        assert!(d.entries[0].service_provider_name.raw().is_empty());
        assert!(d.entries[0].service_name.raw().is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MultilingualServiceNameDescriptor::parse(&[0x5E, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x5E, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = MultilingualServiceNameDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_provider_length_overrun() {
        // provider_len=100 but body tiny.
        let bytes = [TAG, 5, b'e', b'n', b'g', 100, 0];
        let err = MultilingualServiceNameDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_service_length_overrun() {
        // lang + provider_len=0 + service_len=100, no service bytes.
        let bytes = [TAG, 5, b'e', b'n', b'g', 0, 100];
        let err = MultilingualServiceNameDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn empty_descriptor_valid() {
        let d = MultilingualServiceNameDescriptor::parse(&[TAG, 0]).unwrap();
        assert_eq!(d.entries.len(), 0);
    }

    #[test]
    fn serialize_round_trip() {
        let bytes = build(&[
            (*b"eng", b"Provider", b"Channel"),
            (*b"deu", b"Anbieter", b"Sender"),
        ]);
        let parsed = MultilingualServiceNameDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
        let re = MultilingualServiceNameDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = MultilingualServiceNameDescriptor {
            entries: vec![ServiceNameEntry {
                language_code: LangCode(*b"eng"),
                service_provider_name: DvbText::new(b"P"),
                service_name: DvbText::new(b"S"),
            }],
        };
        let mut tiny = [0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_range_provider_name() {
        let provider = vec![0u8; 256];
        let d = MultilingualServiceNameDescriptor {
            entries: vec![ServiceNameEntry {
                language_code: LangCode(*b"eng"),
                service_provider_name: DvbText::new(&provider),
                service_name: DvbText::new(b"S"),
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::FieldOverflow(_)));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_is_stable() {
        let d = MultilingualServiceNameDescriptor {
            entries: vec![ServiceNameEntry {
                language_code: LangCode(*b"eng"),
                service_provider_name: DvbText::new(b"BBC"),
                service_name: DvbText::new(b"One"),
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"language_code\""));
        assert!(json.contains("\"eng\""));
        assert!(json.contains("\"One\""));
        assert!(json.contains("\"BBC\""));
    }
}
