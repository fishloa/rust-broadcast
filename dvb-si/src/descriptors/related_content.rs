//! Related Content Descriptor — ETSI TS 102 323 §10.4.1, Table 108 (tag 0x74).
//!
//! Carried in a PMT ES_info loop to flag that the elementary stream delivers
//! a Related Content Table (RCT) sub_table. The descriptor body is empty — it
//! is a pure marker. Per the TVA PDF (etsi_ts_102_323_v01.04.01, p. 94,
//! Table 108): the only fields are `descriptor_tag` and `descriptor_length`,
//! and "descriptor_length ... shall be set to the number of bytes that follow
//! it" (i.e. zero). At most one related_content descriptor per PMT sub_table.

use super::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for related_content_descriptor.
pub const TAG: u8 = 0x74;
const HEADER_LEN: usize = 2;

/// Related Content Descriptor.
///
/// A zero-length marker — it carries no payload fields and has no reserved
/// bits (Table 108). A non-conformant sender's body bytes are tolerated on
/// parse and kept in [`trailing`](Self::trailing) so they re-serialize.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RelatedContentDescriptor {
    /// Body bytes after the (empty) defined layout. Empty for a conformant
    /// descriptor; re-emitted verbatim otherwise.
    pub trailing: Vec<u8>,
}

impl<'a> Parse<'a> for RelatedContentDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "RelatedContentDescriptor",
            "unexpected tag for related_content_descriptor",
        )?;
        // Body must be empty per Table 108. Trailing payload bytes are tolerated
        // (forward-compatibility) and preserved so the descriptor round-trips.
        Ok(Self {
            trailing: body.to_vec(),
        })
    }
}

impl Serialize for RelatedContentDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.trailing.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, self.trailing.len())?;
        buf[HEADER_LEN..len].copy_from_slice(&self.trailing);
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for RelatedContentDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "RELATED_CONTENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_marker() {
        let bytes = [TAG, 0];
        let d = RelatedContentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d, RelatedContentDescriptor::default());
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
        let d = RelatedContentDescriptor::default();
        let mut buf = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 2);
        assert_eq!(buf, vec![TAG, 0]);
        assert_eq!(RelatedContentDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = RelatedContentDescriptor::default();
        let mut buf = vec![0u8; 1];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    /// Non-empty body bytes of a non-conformant sender are kept and re-emitted.
    #[test]
    fn trailing_bytes_preserved() {
        let bytes = [TAG, 3, 0xDE, 0xAD, 0xBE];
        let d = RelatedContentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.trailing, [0xDE, 0xAD, 0xBE]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }
}
