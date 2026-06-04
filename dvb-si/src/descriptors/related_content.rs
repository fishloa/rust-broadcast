//! Related Content Descriptor — ETSI TS 102 323 §10.4.1, Table 108 (tag 0x74).
//!
//! Carried in a PMT ES_info loop to flag that the elementary stream delivers
//! a Related Content Table (RCT) sub_table. The descriptor body is empty — it
//! is a pure marker. Per the TVA PDF (etsi_ts_102_323_v01.04.01, p. 94,
//! Table 108): the only fields are `descriptor_tag` and `descriptor_length`,
//! and "descriptor_length ... shall be set to the number of bytes that follow
//! it" (i.e. zero). At most one related_content descriptor per PMT sub_table.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for related_content_descriptor.
pub const TAG: u8 = 0x74;
const HEADER_LEN: usize = 2;

/// Related Content Descriptor.
///
/// A zero-length marker — it carries no payload fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RelatedContentDescriptor;

impl<'a> Parse<'a> for RelatedContentDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "RelatedContentDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for related_content_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "RelatedContentDescriptor body",
            });
        }
        // Body must be empty per Table 108. Trailing payload bytes are ignored
        // (forward-compatibility), consistent with permissive parsing elsewhere.
        Ok(Self)
    }
}

impl Serialize for RelatedContentDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
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
        buf[1] = 0;
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for RelatedContentDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_marker() {
        let bytes = [TAG, 0];
        let d = RelatedContentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d, RelatedContentDescriptor);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            RelatedContentDescriptor::parse(&[0x73, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x73, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_header() {
        assert!(matches!(
            RelatedContentDescriptor::parse(&[TAG]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let bytes = [TAG, 3, 1, 2];
        assert!(matches!(
            RelatedContentDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = RelatedContentDescriptor;
        let mut buf = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 2);
        assert_eq!(buf, vec![TAG, 0]);
        assert_eq!(RelatedContentDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = RelatedContentDescriptor;
        let mut buf = vec![0u8; 1];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = RelatedContentDescriptor;
        let j = serde_json::to_string(&d).unwrap();
        let back: RelatedContentDescriptor = serde_json::from_str(&j).unwrap();
        assert_eq!(back, d);
    }
}
