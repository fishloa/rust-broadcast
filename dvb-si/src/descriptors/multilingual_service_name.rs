//! Multilingual Service Name Descriptor — ETSI EN 300 468 §6.2.25 (tag 0x5D).
//!
//! Table 79 (PDF p. 95). Carried in the SDT. A loop of entries, each carrying
//! an ISO 639-2 language code plus TWO length-prefixed strings: the service
//! provider name and the service name.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for multilingual_service_name_descriptor.
pub const TAG: u8 = 0x5D;
const HEADER_LEN: usize = 2;
const LANG_LEN: usize = 3;
const LEN_FIELD: usize = 1;

/// One localised (provider name, service name) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ServiceNameEntry<'a> {
    /// ISO 639-2 language code.
    pub language_code: [u8; 3],
    /// Raw DVB-encoded service provider name bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_provider_name: &'a [u8],
    /// Raw DVB-encoded service name bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_name: &'a [u8],
}

/// Multilingual Service Name Descriptor (tag 0x5D).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct MultilingualServiceNameDescriptor<'a> {
    /// Localised name pairs in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub entries: Vec<ServiceNameEntry<'a>>,
}

impl<'a> Parse<'a> for MultilingualServiceNameDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "MultilingualServiceNameDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for multilingual_service_name_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "MultilingualServiceNameDescriptor body",
            });
        }
        let mut entries = Vec::new();
        let mut pos = HEADER_LEN;
        while pos < end {
            // lang(3) + provider_name_length(1) must fit.
            if pos + LANG_LEN + LEN_FIELD > end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "entry header runs past descriptor end",
                });
            }
            let language_code = [bytes[pos], bytes[pos + 1], bytes[pos + 2]];
            let provider_len_pos = pos + LANG_LEN;
            let provider_len = bytes[provider_len_pos] as usize;
            let provider_start = provider_len_pos + LEN_FIELD;
            let provider_end = provider_start + provider_len;
            // Need provider name + service_name_length field to still fit.
            if provider_end + LEN_FIELD > end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "service_provider_name_length runs past descriptor end",
                });
            }
            let service_provider_name = &bytes[provider_start..provider_end];
            let service_len = bytes[provider_end] as usize;
            let service_start = provider_end + LEN_FIELD;
            let service_end = service_start + service_len;
            if service_end > end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "service_name_length runs past descriptor end",
                });
            }
            let service_name = &bytes[service_start..service_end];
            entries.push(ServiceNameEntry {
                language_code,
                service_provider_name,
                service_name,
            });
            pos = service_end;
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
                    LANG_LEN + LEN_FIELD + e.service_provider_name.len() + LEN_FIELD
                        + e.service_name.len()
                })
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        for e in &self.entries {
            if e.service_provider_name.len() > u8::MAX as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "service_provider_name exceeds 255 bytes (length is 8-bit)",
                });
            }
            if e.service_name.len() > u8::MAX as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "service_name exceeds 255 bytes (length is 8-bit)",
                });
            }
        }
        let len = self.serialized_len();
        let body = len - HEADER_LEN;
        if body > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "multilingual_service_name_descriptor body exceeds 255 bytes",
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
            buf[pos..pos + LANG_LEN].copy_from_slice(&e.language_code);
            pos += LANG_LEN;
            buf[pos] = e.service_provider_name.len() as u8;
            pos += LEN_FIELD;
            buf[pos..pos + e.service_provider_name.len()]
                .copy_from_slice(e.service_provider_name);
            pos += e.service_provider_name.len();
            buf[pos] = e.service_name.len() as u8;
            pos += LEN_FIELD;
            buf[pos..pos + e.service_name.len()].copy_from_slice(e.service_name);
            pos += e.service_name.len();
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for MultilingualServiceNameDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
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
        assert_eq!(&d.entries[0].language_code, b"eng");
        assert_eq!(d.entries[0].service_provider_name, b"BBC");
        assert_eq!(d.entries[0].service_name, b"One");
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = build(&[(*b"eng", b"Prov", b"Svc"), (*b"fra", b"Fourn", b"Chaine")]);
        let d = MultilingualServiceNameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].service_name, b"Chaine");
    }

    #[test]
    fn parse_empty_names_valid() {
        let bytes = build(&[(*b"deu", b"", b"")]);
        let d = MultilingualServiceNameDescriptor::parse(&bytes).unwrap();
        assert!(d.entries[0].service_provider_name.is_empty());
        assert!(d.entries[0].service_name.is_empty());
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
        let bytes = build(&[(*b"eng", b"Provider", b"Channel"), (*b"deu", b"Anbieter", b"Sender")]);
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
                language_code: *b"eng",
                service_provider_name: b"P",
                service_name: b"S",
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
                language_code: *b"eng",
                service_provider_name: &provider,
                service_name: b"S",
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
        let d = MultilingualServiceNameDescriptor {
            entries: vec![ServiceNameEntry {
                language_code: *b"eng",
                service_provider_name: b"BBC",
                service_name: b"One",
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, serde_json::to_string(&d.clone()).unwrap());
    }
}
