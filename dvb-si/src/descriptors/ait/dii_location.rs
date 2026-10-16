//! DII Location Descriptor — ETSI TS 102 727 V1.1.1 §10.8.3.3, Table 83 (AIT tag
//! 0x0D). PDF p.170, transcribed in `docs/descriptors/ts_102_727/83-*.md`,
//! verified against the rendered page.
//!
//! Zero or one in the "common" or "application" AIT loop, for object-carousel
//! transport. Body: `transport_protocol_label`(8) then 4-byte entries of
//! `reserved_future_use`(1) + `DII_identification`(15) + `association_tag`(16).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for DII_location_descriptor (AIT namespace).
pub const TAG: u8 = 0x0D;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 4;
const DII_IDENTIFICATION_MASK: u16 = 0x7FFF;
const RESERVED_BIT: u16 = 0x8000;

/// One DII location entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DiiLocation {
    /// The 1-bit `reserved_future_use`; parsed value kept, default `true`
    /// (reserved bits set to "1").
    pub reserved_future_use: bool,
    /// `DII_identification`(15) — the identification portion of the DII
    /// `transactionId` (TS 102 727 Table B.33).
    pub dii_identification: u16,
    /// `association_tag`(16) — the elementary-stream connection carrying the DII.
    pub association_tag: u16,
}

impl DiiLocation {
    /// An entry with the spec-default reserved bit.
    #[must_use]
    pub const fn new(dii_identification: u16, association_tag: u16) -> Self {
        Self {
            reserved_future_use: true,
            dii_identification,
            association_tag,
        }
    }
}

/// DII Location Descriptor (AIT tag 0x0D).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DiiLocationDescriptor {
    /// `transport_protocol_label` of the carousel's transport_protocol_descriptor.
    pub transport_protocol_label: u8,
    /// DII locations, most important first.
    pub locations: Vec<DiiLocation>,
}

impl<'a> Parse<'a> for DiiLocationDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DiiLocationDescriptor",
            "unexpected tag for DII_location_descriptor",
        )?;
        let (&transport_protocol_label, rest) =
            body.split_first().ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "DII_location_descriptor missing transport_protocol_label",
            })?;
        if rest.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "DII_location_descriptor entry loop is not a multiple of 4 bytes",
            });
        }
        let locations = rest
            .chunks_exact(ENTRY_LEN)
            .map(|c| {
                let id = u16::from_be_bytes([c[0], c[1]]);
                DiiLocation {
                    reserved_future_use: id & RESERVED_BIT != 0,
                    dii_identification: id & DII_IDENTIFICATION_MASK,
                    association_tag: u16::from_be_bytes([c[2], c[3]]),
                }
            })
            .collect();
        Ok(Self {
            transport_protocol_label,
            locations,
        })
    }
}

impl Serialize for DiiLocationDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + 1 + self.locations.len() * ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2] = self.transport_protocol_label;
        let mut pos = 3;
        for l in &self.locations {
            let id = broadcast_common::len::fit_mask(
                l.dii_identification,
                DII_IDENTIFICATION_MASK,
                "DII_identification",
            )?;
            let word = id | (u16::from(l.reserved_future_use) * RESERVED_BIT);
            buf[pos..pos + 2].copy_from_slice(&word.to_be_bytes());
            buf[pos + 2..pos + ENTRY_LEN].copy_from_slice(&l.association_tag.to_be_bytes());
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for DiiLocationDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DII_LOCATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Table 83: label 2; (reserved=1, id 0x0123, tag 0x0456); (reserved=0, id 0x7FFF, tag 0x0001).
    const BYTES: [u8; 11] = [TAG, 9, 0x02, 0x81, 0x23, 0x04, 0x56, 0x7F, 0xFF, 0x00, 0x01];

    #[test]
    fn parse_fields() {
        let d = DiiLocationDescriptor::parse(&BYTES).unwrap();
        assert_eq!(d.transport_protocol_label, 2);
        assert_eq!(d.locations.len(), 2);
        assert!(d.locations[0].reserved_future_use);
        assert_eq!(d.locations[0].dii_identification, 0x0123);
        assert_eq!(d.locations[0].association_tag, 0x0456);
        assert!(!d.locations[1].reserved_future_use);
        assert_eq!(d.locations[1].dii_identification, 0x7FFF);
    }

    #[test]
    fn round_trip_byte_identical_both_reserved_values() {
        let d = DiiLocationDescriptor::parse(&BYTES).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, BYTES);
        assert_eq!(DiiLocationDescriptor::parse(&out).unwrap(), d);
    }

    #[test]
    fn builder_default_reserved_and_errors() {
        let d = DiiLocationDescriptor {
            transport_protocol_label: 0,
            locations: vec![DiiLocation::new(1, 2)],
        };
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 5, 0, 0x80, 0x01, 0x00, 0x02]);
        assert!(DiiLocationDescriptor::parse(&[TAG, 0]).is_err());
        assert!(DiiLocationDescriptor::parse(&[TAG, 4, 0, 1, 2, 3]).is_err());
        let bad = DiiLocationDescriptor {
            transport_protocol_label: 0,
            locations: vec![DiiLocation::new(0x8000, 0)],
        };
        let mut out = vec![0u8; bad.serialized_len()];
        assert!(bad.serialize_into(&mut out).is_err());
    }
}
