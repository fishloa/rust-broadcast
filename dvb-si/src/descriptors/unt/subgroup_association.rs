//! SSU_subgroup_association_descriptor — ETSI TS 102 006 §9.5.2.8 Table 30
//! (UNT tag 0x0B; rendered PDF p.29 verified).
//!
//! Associates an update group within the SSU carousel with the targeting
//! criteria; complements the SSU_location_descriptor. `descriptor_length` is
//! fixed at 5 and `subgroup_tag` (40 bits) holds the OUI of the defining
//! authority in its most significant 24 bits and, in its least significant
//! 16 bits, the value of the `subgroup_association_descriptor` in the DSI's
//! GroupInfoBytes. The syntax is identical to that carousel descriptor
//! (Table 39), so both use the shared typed
//! [`SubgroupTag`]. UNT
//! operational loop only (Table 19).

use crate::descriptors::carousel::SubgroupTag;
use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for SSU_subgroup_association_descriptor.
pub const TAG: u8 = 0x0B;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 5;

/// SSU_subgroup_association_descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SsuSubgroupAssociationDescriptor {
    /// The 40-bit `subgroup_tag`: OUI (24 MSB) + the 16-bit DSI subgroup value.
    pub subgroup_tag: SubgroupTag,
}

impl<'a> Parse<'a> for SsuSubgroupAssociationDescriptor {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SsuSubgroupAssociationDescriptor",
            "unexpected tag for SSU_subgroup_association_descriptor",
        )?;
        let b: [u8; BODY_LEN] = body.try_into().map_err(|_| Error::InvalidDescriptor {
            tag: TAG,
            reason: "SSU_subgroup_association_descriptor body must be 5 bytes",
        })?;
        Ok(Self {
            subgroup_tag: SubgroupTag::from_bytes(b),
        })
    }
}

impl Serialize for SsuSubgroupAssociationDescriptor {
    type Error = Error;
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

impl crate::traits::DescriptorDef<'_> for SsuSubgroupAssociationDescriptor {
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
        assert_eq!(d.subgroup_tag, SubgroupTag::new(0x0000_1234, 0xBEEF));
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn wrong_length_and_over_range_oui_rejected() {
        // descriptor_length is fixed at 5 (Table 30).
        assert!(SsuSubgroupAssociationDescriptor::parse(&[TAG, 4, 1, 2, 3, 4]).is_err());
        assert!(SsuSubgroupAssociationDescriptor::parse(&[TAG, 6, 1, 2, 3, 4, 5, 6]).is_err());
        let wide = SsuSubgroupAssociationDescriptor {
            subgroup_tag: SubgroupTag::new(0x0100_0000, 0),
        };
        assert!(wide.serialize_into(&mut [0u8; 8]).is_err());
    }
}
