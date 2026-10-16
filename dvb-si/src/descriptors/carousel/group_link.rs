//! Group Link Descriptor — ETSI EN 301 192 V1.7.1 §10.2.9, Table 57
//! (data-carousel tag 0x08). PDF pp.67-68, transcribed in
//! `docs/descriptors/en_301_192/57-*.md`, verified against the rendered pages.
//!
//! Links DII group descriptions that together describe one larger group:
//! `position`(8) and the 32-bit `group_id` of the next description (ignored
//! for the last one).

use super::module_link::LinkPosition;
use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for group_link_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x08;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 5;

/// Group Link Descriptor (carousel tag 0x08).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GroupLinkDescriptor {
    /// `position` of this group description in the chain.
    pub position: LinkPosition,
    /// `group_id` of the next group description; ignored for the last one.
    pub group_id: u32,
}

impl<'a> Parse<'a> for GroupLinkDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "GroupLinkDescriptor",
            "unexpected tag for group_link_descriptor",
        )?;
        if body.len() != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "group_link_descriptor body must be 5 bytes",
            });
        }
        Ok(Self {
            position: LinkPosition::from_u8(body[0]),
            group_id: u32::from_be_bytes([body[1], body[2], body[3], body[4]]),
        })
    }
}

impl Serialize for GroupLinkDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, BODY_LEN)?;
        buf[2] = self.position.to_u8();
        buf[3..7].copy_from_slice(&self.group_id.to_be_bytes());
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for GroupLinkDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "GROUP_LINK";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let bytes = [TAG, 5, 0x01, 0xDE, 0xAD, 0xBE, 0xEF];
        let d = GroupLinkDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.position, LinkPosition::Intermediate);
        assert_eq!(d.group_id, 0xDEAD_BEEF);
        let mut out = [0u8; 7];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn reserved_position_preserved_and_bad_length_rejected() {
        let bytes = [TAG, 5, 0x09, 0, 0, 0, 1];
        let d = GroupLinkDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.position, LinkPosition::Reserved(9));
        let mut out = [0u8; 7];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert!(GroupLinkDescriptor::parse(&[TAG, 4, 0, 0, 0, 0]).is_err());
    }
}
