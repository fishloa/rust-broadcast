//! Subgroup Association Descriptor — ETSI TS 102 006 §9.6.2.1, Table 39
//! (data-carousel tag 0x0B, allocated by EN 301 192 Table 48; DSI
//! `GroupInfoBytes`). PDF p.36 of the vendored V1.4.1 text, transcribed in
//! `docs/descriptors/ts_102_006/39-*.md`, verified against the rendered page.
//!
//! Associates an SSU update group with the UNT targeting criteria. Fixed
//! 5-byte body (`descriptor_length` = 5): the 40-bit `subgroup_tag`, whose
//! most significant 24 bits are the OUI of the defining authority and whose
//! least significant 16 bits equal the SSU_subgroup_association_descriptor's
//! value in the UNT.
//!
//! The UNT-side `SSU_subgroup_association_descriptor` (§9.5.2.8 Table 30,
//! rendered p.29) has the identical syntax (tag 0x0B, `descriptor_length` 5,
//! 40-bit `subgroup_tag`) and the same `subgroup_tag` semantics, so both
//! descriptors share the one typed [`SubgroupTag`] defined here.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for subgroup_association_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x0B;
const HEADER_LEN: usize = 2;
/// Length of the 40-bit `subgroup_tag` (= the whole descriptor body).
pub(crate) const BODY_LEN: usize = 5;
const OUI_MASK: u32 = 0x00FF_FFFF;

/// The 40-bit `subgroup_tag` shared by the carousel
/// `subgroup_association_descriptor` (TS 102 006 Table 39) and the UNT
/// `SSU_subgroup_association_descriptor` (Table 30): a 24-bit OUI (most
/// significant) and a 16-bit subgroup value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SubgroupTag {
    /// Most significant 24 bits: the OUI of the holder under whose authority
    /// the tag is unique.
    pub oui: u32,
    /// Least significant 16 bits: the value that must match between the DSI
    /// and the UNT descriptor.
    pub subgroup: u16,
}

impl SubgroupTag {
    /// Build a tag from its two parts (the OUI is range-checked on serialize).
    #[must_use]
    pub const fn new(oui: u32, subgroup: u16) -> Self {
        Self { oui, subgroup }
    }

    /// Decode the 5 wire bytes.
    #[must_use]
    pub const fn from_bytes(b: [u8; BODY_LEN]) -> Self {
        Self {
            oui: u32::from_be_bytes([0, b[0], b[1], b[2]]),
            subgroup: u16::from_be_bytes([b[3], b[4]]),
        }
    }

    /// Encode to the 5 wire bytes; errors when `oui` exceeds 24 bits.
    pub fn to_bytes(self) -> Result<[u8; BODY_LEN]> {
        let oui = broadcast_common::len::fit_mask(self.oui, OUI_MASK, "subgroup_tag OUI")?;
        let o = oui.to_be_bytes();
        let s = self.subgroup.to_be_bytes();
        Ok([o[1], o[2], o[3], s[0], s[1]])
    }
}

/// Subgroup Association Descriptor (carousel tag 0x0B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubgroupAssociationDescriptor {
    /// The 40-bit `subgroup_tag` (OUI + 16-bit subgroup value).
    pub subgroup_tag: SubgroupTag,
}

impl<'a> Parse<'a> for SubgroupAssociationDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SubgroupAssociationDescriptor",
            "unexpected tag for subgroup_association_descriptor",
        )?;
        let b: [u8; BODY_LEN] = body.try_into().map_err(|_| Error::InvalidDescriptor {
            tag: TAG,
            reason: "subgroup_association_descriptor body must be 5 bytes",
        })?;
        Ok(Self {
            subgroup_tag: SubgroupTag::from_bytes(b),
        })
    }
}

impl Serialize for SubgroupAssociationDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let tag = self.subgroup_tag.to_bytes()?;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, BODY_LEN)?;
        buf[HEADER_LEN..len].copy_from_slice(&tag);
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for SubgroupAssociationDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SUBGROUP_ASSOCIATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // OUI 0x00015A (the DVB SSU reserved OUI) + tag 0x0007.
        let bytes = [TAG, 5, 0x00, 0x01, 0x5A, 0x00, 0x07];
        let d = SubgroupAssociationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.subgroup_tag, SubgroupTag::new(0x0000_015A, 7));
        let mut out = [0u8; 7];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn wrong_length_and_over_range_oui_rejected() {
        assert!(SubgroupAssociationDescriptor::parse(&[TAG, 4, 0, 0, 0, 0]).is_err());
        let d = SubgroupAssociationDescriptor {
            subgroup_tag: SubgroupTag::new(0x0100_0000, 0),
        };
        assert!(d.serialize_into(&mut [0u8; 7]).is_err());
    }
}
