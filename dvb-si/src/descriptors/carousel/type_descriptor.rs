//! Type Descriptor — ETSI EN 301 192 V1.7.1 §10.2.2, Table 50 (data-carousel
//! tag 0x01). PDF p.65, transcribed in
//! `docs/descriptors/en_301_192/50-*.md`, verified against the rendered page.
//!
//! The Media Type (IETF RFC 2045 / RFC 2046, e.g. `"text/plain"`) of a module
//! or group: the whole body is the `text_char` string.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::DvbText;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for type_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x01;
const HEADER_LEN: usize = 2;

/// Type Descriptor (carousel tag 0x01).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TypeDescriptor<'a> {
    /// `text_char` string — the Media Type of the module or group.
    pub media_type: DvbText<'a>,
}

impl<'a> Parse<'a> for TypeDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "TypeDescriptor",
            "unexpected tag for type_descriptor",
        )?;
        Ok(Self {
            media_type: DvbText::new(body),
        })
    }
}

impl Serialize for TypeDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.media_type.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, self.media_type.len())?;
        buf[HEADER_LEN..len].copy_from_slice(self.media_type.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TypeDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TYPE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip_spec_example_text_plain() {
        // Table 50 value column example: "text/plain".
        let mut bytes = vec![TAG, 10];
        bytes.extend_from_slice(b"text/plain");
        let d = TypeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.media_type.raw(), b"text/plain");
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_wrong_tag_truncated_and_overflow() {
        let d = TypeDescriptor::parse(&[TAG, 0]).unwrap();
        assert_eq!(d.serialized_len(), 2);
        assert!(TypeDescriptor::parse(&[0x02, 0]).is_err());
        assert!(TypeDescriptor::parse(&[TAG, 3, b'a']).is_err());
        let big = vec![b'x'; 256];
        let d = TypeDescriptor {
            media_type: DvbText::new(&big),
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
