//! Service Identifier Descriptor — ETSI TS 102 809 §7.2, Table 39 (tag 0x71).
//!
//! Carried in the SDT/service loop to give a service a stable textual
//! identifier. The body is a run of `textual_service_identifier_bytes`
//! (ASCII), per ts_102_809_apps.md "Table 39 — Service identifier descriptor"
//! (PDF p. 62).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for service_identifier_descriptor.
pub const TAG: u8 = 0x71;
const HEADER_LEN: usize = 2;

/// Service Identifier Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ServiceIdentifierDescriptor<'a> {
    /// Raw textual_service_identifier bytes (ASCII).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub textual_service_identifier: &'a [u8],
}

impl<'a> Parse<'a> for ServiceIdentifierDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "ServiceIdentifierDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for service_identifier_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "ServiceIdentifierDescriptor body",
            });
        }
        Ok(Self {
            textual_service_identifier: &bytes[HEADER_LEN..end],
        })
    }
}

impl Serialize for ServiceIdentifierDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.textual_service_identifier.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.textual_service_identifier.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "textual_service_identifier exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = self.descriptor_length();
        buf[HEADER_LEN..len].copy_from_slice(self.textual_service_identifier);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for ServiceIdentifierDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        self.textual_service_identifier.len() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_identifier_bytes() {
        let bytes = [TAG, 6, b'B', b'B', b'C', b'O', b'N', b'E'];
        let d = ServiceIdentifierDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.textual_service_identifier, b"BBCONE");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            ServiceIdentifierDescriptor::parse(&[0x70, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x70, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_header() {
        assert!(matches!(
            ServiceIdentifierDescriptor::parse(&[TAG]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let bytes = [TAG, 5, 1, 2, 3];
        assert!(matches!(
            ServiceIdentifierDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn empty_identifier_is_valid() {
        let bytes = [TAG, 0];
        let d = ServiceIdentifierDescriptor::parse(&bytes).unwrap();
        assert!(d.textual_service_identifier.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = ServiceIdentifierDescriptor {
            textual_service_identifier: b"CH4-HD",
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(ServiceIdentifierDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = ServiceIdentifierDescriptor {
            textual_service_identifier: b"test",
        };
        let mut buf = vec![0u8; 1];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serializes_to_stable_json() {
        // Borrowed `&[u8]` cannot deserialize from a JSON number array (serde
        // has no JSON byte type), so we assert the Serialize impl is wired and
        // emits stable, re-parseable JSON rather than attempting a borrow
        // round-trip through JSON.
        let d = ServiceIdentifierDescriptor {
            textual_service_identifier: b"ITV1",
        };
        let j = serde_json::to_string(&d).unwrap();
        // Valid, re-parseable JSON (key order is map-defined, so we do not
        // assert byte-for-byte string stability).
        let _v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert!(j.contains("textual_service_identifier"));
    }
}
