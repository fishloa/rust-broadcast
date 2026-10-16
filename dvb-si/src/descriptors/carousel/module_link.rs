//! Module Link Descriptor — ETSI EN 301 192 V1.7.1 §10.2.5, Table 53
//! (data-carousel tag 0x04). PDF p.66, transcribed in
//! `docs/descriptors/en_301_192/53-*.md`, verified against the rendered page.
//!
//! Chains modules into one piece of data: `position`(8) and the `module_id`(16)
//! of the next module (ignored for the last one). [`LinkPosition`] is shared
//! with the group-link descriptor (§10.2.9, same value coding).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for module_link_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x04;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 3;

/// `position` of a module / group description in its chain
/// (EN 301 192 §10.2.5 / §10.2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LinkPosition {
    /// 0x00 — first of the list.
    First,
    /// 0x01 — an intermediate entry.
    Intermediate,
    /// 0x02 — last of the list.
    Last,
    /// Any other value (reserved).
    Reserved(u8),
}

impl LinkPosition {
    /// Decode from the 8-bit wire value.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::First,
            0x01 => Self::Intermediate,
            0x02 => Self::Last,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 8-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::First => 0x00,
            Self::Intermediate => 0x01,
            Self::Last => 0x02,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Intermediate => "intermediate",
            Self::Last => "last",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(LinkPosition, Reserved);

/// Module Link Descriptor (carousel tag 0x04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ModuleLinkDescriptor {
    /// `position` of this module in the chain.
    pub position: LinkPosition,
    /// `module_id` of the next module in the list; ignored for the last one.
    pub module_id: u16,
}

impl<'a> Parse<'a> for ModuleLinkDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ModuleLinkDescriptor",
            "unexpected tag for module_link_descriptor",
        )?;
        if body.len() != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "module_link_descriptor body must be 3 bytes",
            });
        }
        Ok(Self {
            position: LinkPosition::from_u8(body[0]),
            module_id: u16::from_be_bytes([body[1], body[2]]),
        })
    }
}

impl Serialize for ModuleLinkDescriptor {
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
        buf[3..5].copy_from_slice(&self.module_id.to_be_bytes());
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for ModuleLinkDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MODULE_LINK";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip_each_position() {
        for (pos, p) in [
            (0u8, LinkPosition::First),
            (1, LinkPosition::Intermediate),
            (2, LinkPosition::Last),
            (7, LinkPosition::Reserved(7)),
        ] {
            let bytes = [TAG, 3, pos, 0x12, 0x34];
            let d = ModuleLinkDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.position, p);
            assert_eq!(d.module_id, 0x1234);
            let mut out = [0u8; 5];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn wrong_length_rejected() {
        assert!(ModuleLinkDescriptor::parse(&[TAG, 2, 0, 0]).is_err());
        assert!(ModuleLinkDescriptor::parse(&[TAG, 4, 0, 0, 0, 0]).is_err());
        assert!(ModuleLinkDescriptor::parse(&[TAG, 3, 0, 0]).is_err());
    }
}
