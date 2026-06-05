//! Service Descriptor — ETSI EN 300 468 §6.2.33 (tag 0x48).
//!
//! Carried inside SDT. Provides the provider and service name plus a
//! service_type byte classifying the service (TV SD, TV HD, radio, data, …).

use crate::error::{Error, Result};
use crate::text::DvbText;
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for service_descriptor.
pub const TAG: u8 = 0x48;
const HEADER_LEN: usize = 2;

/// Service Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ServiceDescriptor<'a> {
    /// service_type byte (ETSI Table 87).
    pub service_type: u8,
    /// DVB Annex-A encoded provider name.
    pub provider_name: DvbText<'a>,
    /// DVB Annex-A encoded service name.
    pub service_name: DvbText<'a>,
}

impl<'a> Parse<'a> for ServiceDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "ServiceDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for service_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "ServiceDescriptor body",
            });
        }
        if length < 3 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_descriptor body too short for service_type + two length fields",
            });
        }
        let service_type = bytes[HEADER_LEN];
        let provider_len = bytes[HEADER_LEN + 1] as usize;
        let provider_end = HEADER_LEN + 2 + provider_len;
        if provider_end + 1 > end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_provider_name_length runs past descriptor end",
            });
        }
        let provider_name = DvbText::new(&bytes[HEADER_LEN + 2..provider_end]);
        let service_len = bytes[provider_end] as usize;
        let service_end = provider_end + 1 + service_len;
        if service_end > end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_name_length runs past descriptor end",
            });
        }
        let service_name = DvbText::new(&bytes[provider_end + 1..service_end]);
        Ok(Self {
            service_type,
            provider_name,
            service_name,
        })
    }
}

impl Serialize for ServiceDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + 1 + 1 + self.provider_name.len() + 1 + self.service_name.len()
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
        buf[2] = self.service_type;
        buf[3] = self.provider_name.len() as u8;
        let p_start = 4;
        let p_end = p_start + self.provider_name.len();
        buf[p_start..p_end].copy_from_slice(self.provider_name.raw());
        buf[p_end] = self.service_name.len() as u8;
        let s_start = p_end + 1;
        buf[s_start..s_start + self.service_name.len()].copy_from_slice(self.service_name.raw());
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for ServiceDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ServiceDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SERVICE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_all_fields() {
        // service_type=1, provider="EUTE", service="TF1"
        let bytes = [
            TAG, 10, 0x01, 4, b'E', b'U', b'T', b'E', 3, b'T', b'F', b'1',
        ];
        let d = ServiceDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.service_type, 1);
        assert_eq!(d.provider_name.raw(), b"EUTE");
        assert_eq!(d.service_name.raw(), b"TF1");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = ServiceDescriptor::parse(&[0x49, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x49, .. }));
    }

    #[test]
    fn parse_rejects_short_header() {
        let err = ServiceDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        let err = ServiceDescriptor::parse(&[TAG, 5, 0x01, 0xFF]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_provider_length_overrun() {
        // provider_len says 100 but descriptor body only 5 bytes.
        let bytes = [TAG, 5, 0x01, 100, b'A', b'B', b'C'];
        let err = ServiceDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn empty_provider_and_service_names_valid() {
        let bytes = [TAG, 3, 0x01, 0, 0];
        let d = ServiceDescriptor::parse(&bytes).unwrap();
        assert!(d.provider_name.raw().is_empty());
        assert!(d.service_name.raw().is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = ServiceDescriptor {
            service_type: 0x19,
            provider_name: DvbText::new(b"BBC"),
            service_name: DvbText::new(b"BBC ONE HD"),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ServiceDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = ServiceDescriptor {
            service_type: 1,
            provider_name: DvbText::new(b"AA"),
            service_name: DvbText::new(b"BBB"),
        };
        // 1 (type) + 1 (p_len) + 2 (p) + 1 (s_len) + 3 (s) = 8
        assert_eq!(d.descriptor_length(), 8);
    }
}
