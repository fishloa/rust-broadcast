//! Copyright Descriptor — ISO/IEC 13818-1 §2.6.24 (tag 0x0D).
//!
//! Carries a 32-bit copyright identifier and optional additional
//! copyright information bytes.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for copyright_descriptor.
pub const TAG: u8 = 0x0D;
const HEADER_LEN: usize = 2;
const COPYRIGHT_ID_LEN: usize = 4;

/// Copyright Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CopyrightDescriptor<'a> {
    /// 32-bit copyright identifier.
    pub copyright_identifier: u32,
    /// Additional copyright info bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub additional_copyright_info: &'a [u8],
}

impl<'a> Parse<'a> for CopyrightDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "CopyrightDescriptor",
            "unexpected tag for copyright_descriptor",
        )?;
        if body.len() < COPYRIGHT_ID_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "copyright_descriptor length too short for copyright_identifier",
            });
        }
        let copyright_identifier = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
        let additional_copyright_info = &body[COPYRIGHT_ID_LEN..];
        Ok(Self {
            copyright_identifier,
            additional_copyright_info,
        })
    }
}

impl Serialize for CopyrightDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + COPYRIGHT_ID_LEN + self.additional_copyright_info.len()
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
        buf[HEADER_LEN..HEADER_LEN + COPYRIGHT_ID_LEN]
            .copy_from_slice(&self.copyright_identifier.to_be_bytes());
        buf[HEADER_LEN + COPYRIGHT_ID_LEN..len].copy_from_slice(self.additional_copyright_info);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for CopyrightDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "COPYRIGHT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_id_only() {
        let bytes = [TAG, 4, 0x12, 0x34, 0x56, 0x78];
        let d = CopyrightDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.copyright_identifier, 0x12345678);
        assert!(d.additional_copyright_info.is_empty());
    }

    #[test]
    fn parse_with_additional_info() {
        let bytes = [TAG, 6, 0xDE, 0xAD, 0xBE, 0xEF, 0xAA, 0xBB];
        let d = CopyrightDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.copyright_identifier, 0xDEADBEEF);
        assert_eq!(d.additional_copyright_info, &[0xAA, 0xBB]);
    }

    #[test]
    fn serialize_round_trip() {
        let d = CopyrightDescriptor {
            copyright_identifier: 0xCAFEBABE,
            additional_copyright_info: &[0x01, 0x02, 0x03],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = CopyrightDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = CopyrightDescriptor::parse(&[0x0E, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x0E, .. }));
    }

    #[test]
    fn parse_rejects_too_short_for_id() {
        let err = CopyrightDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = CopyrightDescriptor {
            copyright_identifier: 0,
            additional_copyright_info: &[],
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
