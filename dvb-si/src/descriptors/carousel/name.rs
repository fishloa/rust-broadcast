//! Name Descriptor — ETSI EN 301 192 V1.7.1 §10.2.3, Table 51 (data-carousel
//! tag 0x02). PDF p.65, transcribed in `private/specs-verified/dvb/dvb-si/descriptors/en_301_192/51-*.md`,
//! verified against the rendered page.
//!
//! The name of a module or group (e.g. `"index"`); the whole body is the
//! `text_char` string, coded with the EN 300 468 Annex A character sets.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::Result;
use crate::text::DvbText;
use broadcast_common::len::need_out;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for name_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x02;
const HEADER_LEN: usize = 2;

/// Name Descriptor (carousel tag 0x02).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NameDescriptor<'a> {
    /// `text_char` string — name of the module or group.
    pub name: DvbText<'a>,
}

impl<'a> Parse<'a> for NameDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "NameDescriptor",
            "unexpected tag for name_descriptor",
        )?;
        Ok(Self {
            name: DvbText::new(body),
        })
    }
}

impl Serialize for NameDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.name.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        write_descriptor_header(buf, TAG, self.name.len())?;
        buf[HEADER_LEN..len].copy_from_slice(self.name.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for NameDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "NAME";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn parse_and_round_trip_spec_example_index() {
        // Table 51 value column example: "index".
        let bytes = [TAG, 5, b'i', b'n', b'd', b'e', b'x'];
        let d = NameDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.name.raw(), b"index");
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn wrong_tag_truncated_and_overflow() {
        assert!(matches!(
            NameDescriptor::parse(&[0x01, 0]),
            Err(Error::InvalidDescriptor { tag: 0x01, .. })
        ));
        assert!(matches!(
            NameDescriptor::parse(&[TAG, 3, b'a']),
            Err(Error::BufferTooShort(..))
        ));
        let big = vec![b'x'; 256];
        let d = NameDescriptor {
            name: DvbText::new(&big),
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut out),
            Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                field: "descriptor_length",
                ..
            }))
        ));
    }
}
