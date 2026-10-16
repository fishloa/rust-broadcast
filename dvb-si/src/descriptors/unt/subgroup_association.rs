//! SSU_subgroup_association_descriptor — ETSI TS 102 006 §9.5.2.8 Table 30
//! (UNT tag 0x0B).
//!
//! Associates an update group within the SSU carousel with the targeting
//! criteria; complements the SSU_location_descriptor. `subgroup_tag` (40
//! bits) holds the OUI of the defining authority in its most significant 24
//! bits and, in its least significant 16 bits, the value of the
//! `subgroup_association_descriptor` in the DSI's GroupInfoBytes. UNT
//! operational loop only (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for SSU_subgroup_association_descriptor.
pub const TAG: u8 = 0x0B;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 5;
const SUBGROUP_TAG_BITS: u32 = 40;

/// SSU_subgroup_association_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SsuSubgroupAssociationDescriptor<'a> {
    /// `subgroup_tag` (40 bits in the low bits of a `u64`): OUI (24 MSB) +
    /// the 16-bit DSI subgroup value.
    pub subgroup_tag: u64,
    /// Bytes after the 5 defined body bytes inside `descriptor_length`; the
    /// spec fixes `descriptor_length` at 5 (kept for byte-identical
    /// re-serialization of a non-conformant sender).
    pub trailing: &'a [u8],
}

impl SsuSubgroupAssociationDescriptor<'_> {
    /// The 24-bit OUI of the defining authority (the 24 MSBs of `subgroup_tag`).
    #[must_use]
    pub const fn oui(&self) -> u32 {
        let b = self.subgroup_tag.to_be_bytes();
        u32::from_be_bytes([0, b[3], b[4], b[5]])
    }

    /// The 16-bit subgroup value (the 16 LSBs of `subgroup_tag`).
    #[must_use]
    pub const fn subgroup(&self) -> u16 {
        let b = self.subgroup_tag.to_be_bytes();
        u16::from_be_bytes([b[6], b[7]])
    }
}

impl<'a> Parse<'a> for SsuSubgroupAssociationDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SsuSubgroupAssociationDescriptor",
            "unexpected tag for SSU_subgroup_association_descriptor",
        )?;
        let (f, trailing) =
            body.split_first_chunk::<BODY_LEN>()
                .ok_or(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "SSU_subgroup_association_descriptor body shorter than 5 bytes",
                })?;
        let mut tag8 = [0u8; 8];
        tag8[8 - BODY_LEN..].copy_from_slice(f);
        Ok(Self {
            subgroup_tag: u64::from_be_bytes(tag8),
            trailing,
        })
    }
}

impl Serialize for SsuSubgroupAssociationDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let tag40 =
            broadcast_common::len::fit_bits(self.subgroup_tag, SUBGROUP_TAG_BITS, "subgroup_tag")?;
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2..2 + BODY_LEN].copy_from_slice(&tag40.to_be_bytes()[8 - BODY_LEN..]);
        buf[HEADER_LEN + BODY_LEN..len].copy_from_slice(self.trailing);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for SsuSubgroupAssociationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SSU_SUBGROUP_ASSOCIATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_oui_subgroup_and_round_trip() {
        let bytes = [TAG, 5, 0x00, 0x12, 0x34, 0xBE, 0xEF];
        let d = SsuSubgroupAssociationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.oui(), 0x0000_1234);
        assert_eq!(d.subgroup(), 0xBEEF);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn trailing_round_trip_and_errors() {
        let bytes = [TAG, 6, 1, 2, 3, 4, 5, 0x77];
        let d = SsuSubgroupAssociationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.trailing, &[0x77]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert!(SsuSubgroupAssociationDescriptor::parse(&[TAG, 4, 1, 2, 3, 4]).is_err());
        let wide = SsuSubgroupAssociationDescriptor {
            subgroup_tag: 1 << 40,
            trailing: &[],
        };
        assert!(wide.serialize_into(&mut [0u8; 8]).is_err());
    }
}
