//! Deferred Association Tags Descriptor — ETSI EN 301 192 V1.7.1 §11.3.3,
//! Tables 64 and 65 (tag 0x15; "the decimal value of 21"). PDF p.73, transcribed
//! in `docs/descriptors/en_301_192/64-*.md`, verified against the rendered page.
//!
//! Carried in the PMT (`ES_info` loop of the data-broadcast stream) to list
//! `association_tag`s that belong to a stream outside this data broadcast
//! service, or to another DVB service, together with that service's
//! `transport_stream_id` and `program_number` (= `service_id`). TS 102 727 / TS 102 809
//! B.3.3 do not restate the layout; they add that a `transport_stream_id` of
//! 0x0000 "shall be ignored in resolving the reference" (kept verbatim here).
//!
//! Wire layout:
//!
//! ```text
//! deferred_association_tags_descriptor() {
//!   descriptor_tag               8   = 0x15
//!   descriptor_length            8
//!   association_tags_loop_length 8   (bytes; 2 per association_tag)
//!   for (i=0; i<N; i++) { association_tag 16 }
//!   transport_stream_id         16
//!   program_number              16
//!   for (i=0; i<N; i++) { private_data_byte 8 }   // = deferred_service_location()
//! }
//! deferred_service_location() {                    // Table 65
//!   org_network_id              16
//!   for (i=0; i<N; i++) { private_data_byte 8 }    // "not specified by the present document"
//! }
//! ```
//!
//! `private_data_byte` "shall contain the deferred_service_location structure",
//! so the typed field is [`DeferredServiceLocation`]; it is `None` when a
//! descriptor carries no private bytes at all (tolerated, round-trips as
//! such). A single private byte cannot hold the 16-bit `org_network_id` and is
//! rejected.

use super::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for deferred_association_tags_descriptor.
pub const TAG: u8 = 0x15;

/// 2-byte descriptor outer header (tag + length).
const HEADER_LEN: usize = 2;
/// `association_tags_loop_length` field width in bytes (8-bit uimsbf).
const LOOP_LENGTH_LEN: usize = 1;
/// One `association_tag` (16-bit uimsbf).
const ASSOCIATION_TAG_LEN: usize = 2;
/// `transport_stream_id` + `program_number` (2 × 16-bit uimsbf).
const SERVICE_ID_LEN: usize = 4;
/// `org_network_id` field width in bytes (16-bit uimsbf).
const ORG_NETWORK_ID_LEN: usize = 2;

/// `deferred_service_location()` — Table 65.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DeferredServiceLocation<'a> {
    /// `org_network_id` — the `network_id` of the delivery system from which the
    /// service originates.
    pub org_network_id: u16,
    /// `private_data_byte` — "not specified by the present document".
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

impl<'a> DeferredServiceLocation<'a> {
    /// Build a `deferred_service_location()`.
    #[must_use]
    pub const fn new(org_network_id: u16, private_data: &'a [u8]) -> Self {
        Self {
            org_network_id,
            private_data,
        }
    }

    fn serialized_len(&self) -> usize {
        ORG_NETWORK_ID_LEN + self.private_data.len()
    }
}

/// Deferred Association Tags Descriptor (tag 0x15) — EN 301 192 §11.3.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct DeferredAssociationTagsDescriptor<'a> {
    /// `association_tag` loop, in wire order.
    pub association_tags: Vec<u16>,
    /// `transport_stream_id` of the TS in which the associated service resides.
    pub transport_stream_id: u16,
    /// `program_number` — the `service_id` of the associated service.
    pub program_number: u16,
    /// The `private_data_byte` tail, i.e. the `deferred_service_location()`
    /// structure (Table 65); `None` when the descriptor ends after
    /// `program_number`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub deferred_service_location: Option<DeferredServiceLocation<'a>>,
}

impl<'a> DeferredAssociationTagsDescriptor<'a> {
    /// Build a descriptor from its typed fields.
    #[must_use]
    pub fn new(
        association_tags: Vec<u16>,
        transport_stream_id: u16,
        program_number: u16,
        deferred_service_location: Option<DeferredServiceLocation<'a>>,
    ) -> Self {
        Self {
            association_tags,
            transport_stream_id,
            program_number,
            deferred_service_location,
        }
    }

    fn body_len(&self) -> usize {
        LOOP_LENGTH_LEN
            + self.association_tags.len() * ASSOCIATION_TAG_LEN
            + SERVICE_ID_LEN
            + self
                .deferred_service_location
                .as_ref()
                .map_or(0, DeferredServiceLocation::serialized_len)
    }
}

impl<'a> Parse<'a> for DeferredAssociationTagsDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DeferredAssociationTagsDescriptor",
            "unexpected tag for deferred_association_tags_descriptor",
        )?;
        let (&loop_len, rest) = body.split_first().ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "deferred_association_tags_descriptor missing association_tags_loop_length",
        })?;
        let loop_len = usize::from(loop_len);
        if loop_len % ASSOCIATION_TAG_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "association_tags_loop_length is not a multiple of 2",
            });
        }
        if rest.len() < loop_len + SERVICE_ID_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "deferred_association_tags_descriptor body shorter than its loop and service ids",
            });
        }
        let (tags, rest) = rest.split_at(loop_len);
        let association_tags = tags
            .chunks_exact(ASSOCIATION_TAG_LEN)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        let transport_stream_id = u16::from_be_bytes([rest[0], rest[1]]);
        let program_number = u16::from_be_bytes([rest[2], rest[3]]);
        let private = &rest[SERVICE_ID_LEN..];
        let deferred_service_location = match private.len() {
            0 => None,
            n if n < ORG_NETWORK_ID_LEN => {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "deferred_service_location shorter than org_network_id",
                });
            }
            _ => Some(DeferredServiceLocation {
                org_network_id: u16::from_be_bytes([private[0], private[1]]),
                private_data: &private[ORG_NETWORK_ID_LEN..],
            }),
        };
        Ok(Self {
            association_tags,
            transport_stream_id,
            program_number,
            deferred_service_location,
        })
    }
}

impl Serialize for DeferredAssociationTagsDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, self.body_len())?;
        let mut pos = HEADER_LEN;
        buf[pos] = broadcast_common::len::fit_u8(
            self.association_tags.len() * ASSOCIATION_TAG_LEN,
            "association_tags_loop_length",
        )?;
        pos += LOOP_LENGTH_LEN;
        for tag in &self.association_tags {
            buf[pos..pos + ASSOCIATION_TAG_LEN].copy_from_slice(&tag.to_be_bytes());
            pos += ASSOCIATION_TAG_LEN;
        }
        buf[pos..pos + 2].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[pos + 2..pos + SERVICE_ID_LEN].copy_from_slice(&self.program_number.to_be_bytes());
        pos += SERVICE_ID_LEN;
        if let Some(loc) = &self.deferred_service_location {
            buf[pos..pos + ORG_NETWORK_ID_LEN].copy_from_slice(&loc.org_network_id.to_be_bytes());
            pos += ORG_NETWORK_ID_LEN;
            buf[pos..pos + loc.private_data.len()].copy_from_slice(loc.private_data);
        }
        Ok(total)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for DeferredAssociationTagsDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DEFERRED_ASSOCIATION_TAGS";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Spec vector only (no real instance exists in the committed corpus).
    // Hand-built from Tables 64/65:
    //   [0]    15          descriptor_tag (decimal 21)
    //   [1]    0D          descriptor_length = 13
    //   [2]    04          association_tags_loop_length = 4 bytes
    //   [3..5] 00 0A       association_tag 0x000A
    //   [5..7] 01 02       association_tag 0x0102
    //   [7..9] 12 34       transport_stream_id 0x1234
    //   [9..11] 56 78      program_number 0x5678
    //   [11..13] 00 22     deferred_service_location.org_network_id 0x0022
    //   [13..15] AA BB     deferred_service_location private_data_byte
    const BYTES: [u8; 15] = [
        0x15, 0x0D, 0x04, 0x00, 0x0A, 0x01, 0x02, 0x12, 0x34, 0x56, 0x78, 0x00, 0x22, 0xAA, 0xBB,
    ];

    #[test]
    fn parse_and_round_trip() {
        let d = DeferredAssociationTagsDescriptor::parse(&BYTES).unwrap();
        assert_eq!(d.association_tags, [0x000A, 0x0102]);
        assert_eq!(d.transport_stream_id, 0x1234);
        assert_eq!(d.program_number, 0x5678);
        let loc = d.deferred_service_location.as_ref().unwrap();
        assert_eq!(loc.org_network_id, 0x0022);
        assert_eq!(loc.private_data, [0xAA, 0xBB]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, BYTES);
    }

    #[test]
    fn no_private_bytes_round_trips() {
        // empty tag loop, ids only: 15 05 00 00 01 00 02
        let bytes = [0x15, 0x05, 0x00, 0x00, 0x01, 0x00, 0x02];
        let d = DeferredAssociationTagsDescriptor::parse(&bytes).unwrap();
        assert!(d.association_tags.is_empty());
        assert_eq!((d.transport_stream_id, d.program_number), (1, 2));
        assert!(d.deferred_service_location.is_none());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn malformed_is_rejected() {
        // odd association_tags_loop_length
        assert!(
            DeferredAssociationTagsDescriptor::parse(&[
                0x15, 0x08, 0x03, 0, 1, 2, 0, 1, 0, 2
            ])
            .is_err()
        );
        // loop longer than body
        assert!(
            DeferredAssociationTagsDescriptor::parse(&[0x15, 0x05, 0x04, 0, 1, 0, 2]).is_err()
        );
        // one private byte cannot hold org_network_id
        assert!(
            DeferredAssociationTagsDescriptor::parse(&[0x15, 0x06, 0x00, 0, 1, 0, 2, 0xFF])
                .is_err()
        );
        assert!(DeferredAssociationTagsDescriptor::parse(&[0x14, 0x00]).is_err());
        assert!(DeferredAssociationTagsDescriptor::parse(&[0x15, 0x00]).is_err());
    }

    #[test]
    fn over_range_loop_is_an_error_not_a_wrap() {
        let d = DeferredAssociationTagsDescriptor::new(vec![0; 128], 0, 0, None);
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
