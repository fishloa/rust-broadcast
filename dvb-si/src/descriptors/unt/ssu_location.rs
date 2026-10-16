//! SSU_location_descriptor — ETSI TS 102 006 §9.5.2.7 Table 29 (UNT tag 0x03).
//!
//! Locates the SSU data carousel: `data_broadcast_id` (ETSI TS 101 162; 0x000A
//! is the standard SSU two-layer data carousel), an `association_tag` present
//! only for 0x000A, and trailing `private_data_byte`s. UNT common and
//! operational loops (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for SSU_location_descriptor.
pub const TAG: u8 = 0x03;
const HEADER_LEN: usize = 2;
const ID_LEN: usize = 2;
const TAG_LEN: usize = 2;
/// `data_broadcast_id` of the standard SSU two-layer data carousel
/// (TS 102 006 §9.5.2.7; ETSI TS 101 162).
pub const DATA_BROADCAST_ID_SSU: u16 = 0x000A;

/// SSU_location_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SsuLocationDescriptor<'a> {
    /// `data_broadcast_id` — the transport mechanism (ETSI TS 101 162).
    pub data_broadcast_id: u16,
    /// `association_tag` — present iff `data_broadcast_id == 0x000A`.
    pub association_tag: Option<u16>,
    /// `private_data_byte`s — opaque operator data.
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for SsuLocationDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SsuLocationDescriptor",
            "unexpected tag for SSU_location_descriptor",
        )?;
        let (id, rest) = body
            .split_first_chunk::<ID_LEN>()
            .ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "SSU_location_descriptor shorter than data_broadcast_id",
            })?;
        let data_broadcast_id = u16::from_be_bytes(*id);
        if data_broadcast_id != DATA_BROADCAST_ID_SSU {
            return Ok(Self {
                data_broadcast_id,
                association_tag: None,
                private_data: rest,
            });
        }
        let (at, private_data) =
            rest.split_first_chunk::<TAG_LEN>()
                .ok_or(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "SSU_location_descriptor 0x000A shorter than association_tag",
                })?;
        Ok(Self {
            data_broadcast_id,
            association_tag: Some(u16::from_be_bytes(*at)),
            private_data,
        })
    }
}

impl Serialize for SsuLocationDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ID_LEN + self.association_tag.map_or(0, |_| TAG_LEN) + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        // association_tag exists on the wire iff data_broadcast_id == 0x000A.
        if self.association_tag.is_some() != (self.data_broadcast_id == DATA_BROADCAST_ID_SSU) {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "association_tag must be present iff data_broadcast_id == 0x000A",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2..4].copy_from_slice(&self.data_broadcast_id.to_be_bytes());
        let mut off = 4;
        if let Some(at) = self.association_tag {
            buf[off..off + TAG_LEN].copy_from_slice(&at.to_be_bytes());
            off += TAG_LEN;
        }
        buf[off..len].copy_from_slice(self.private_data);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for SsuLocationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SSU_LOCATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssu_carousel_with_association_tag_round_trip() {
        let bytes = [TAG, 5, 0x00, 0x0A, 0x01, 0x23, 0x99];
        let d = SsuLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.data_broadcast_id, DATA_BROADCAST_ID_SSU);
        assert_eq!(d.association_tag, Some(0x0123));
        assert_eq!(d.private_data, &[0x99]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn other_id_has_no_association_tag_round_trip() {
        let bytes = [TAG, 4, 0x00, 0x05, 0xAA, 0xBB];
        let d = SsuLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.association_tag, None);
        assert_eq!(d.private_data, &[0xAA, 0xBB]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn rejects_truncated_and_inconsistent() {
        assert!(SsuLocationDescriptor::parse(&[TAG, 1, 0]).is_err());
        assert!(SsuLocationDescriptor::parse(&[TAG, 3, 0x00, 0x0A, 0x01]).is_err());
        let d = SsuLocationDescriptor {
            data_broadcast_id: 0x0005,
            association_tag: Some(1),
            private_data: &[],
        };
        let mut out = vec![0u8; 16];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
