//! Service Availability Descriptor — ETSI EN 300 468 §6.2.36 (tag 0x72, Table 90, PDF p. 101).
//!
//! Carried inside the SDT. Body layout (Table 90):
//! `availability_flag` (1 bit) + `reserved_future_use` (7 bits), then a
//! `for (i=0;i<N;i++) { cell_id (16) }` loop listing the cells in which the
//! service is (un)available. `availability_flag`=1 → service available in the
//! listed cells; =0 → unavailable in the listed cells.

use super::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for service_availability_descriptor.
pub const TAG: u8 = 0x72;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Length of the flags byte (availability_flag + reserved).
pub const FLAGS_LEN: usize = 1;
/// Length of one cell_id entry.
pub const CELL_ID_LEN: usize = 2;

const AVAILABILITY_FLAG_MASK: u8 = 0b1000_0000;
const RESERVED_MASK: u8 = 0b0111_1111;
/// Spec default of `reserved_future_use`(7): all ones (EN 300 468 V1.5.1-V1.20.1
/// Table 90 (`reserved` before V1.17.1); audit
/// `dvb-si/docs/reserved-bits-edition-audit.md` §6.1, class B: renamed only).
pub const RESERVED_FUTURE_USE_DEFAULT: u8 = 0x7F;

/// Service Availability Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ServiceAvailabilityDescriptor {
    /// `reserved_future_use`(7) after `availability_flag` (7 bits,
    /// right-aligned). Captured on parse and re-emitted verbatim;
    /// [`ServiceAvailabilityDescriptor::new`] defaults to
    /// [`RESERVED_FUTURE_USE_DEFAULT`].
    pub reserved_future_use: u8,
    /// `availability_flag`: true = available in listed cells, false = unavailable.
    pub availability_flag: bool,
    /// cell_id entries in wire order.
    pub cell_ids: Vec<u16>,
}

impl ServiceAvailabilityDescriptor {
    /// Build a descriptor with the spec-default reserved bits (all ones).
    #[must_use]
    pub const fn new(availability_flag: bool, cell_ids: Vec<u16>) -> Self {
        Self {
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
            availability_flag,
            cell_ids,
        }
    }
}

impl<'a> Parse<'a> for ServiceAvailabilityDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ServiceAvailabilityDescriptor",
            "unexpected tag for service_availability_descriptor",
        )?;
        if body.len() < FLAGS_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_availability_descriptor body too short (need flags byte)",
            });
        }
        if !(body.len() - FLAGS_LEN).is_multiple_of(CELL_ID_LEN) {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_availability cell_id loop must be a multiple of 2 bytes",
            });
        }
        let flags = body[0];
        let availability_flag = flags & AVAILABILITY_FLAG_MASK != 0;
        let cell_data = &body[FLAGS_LEN..];
        let count = cell_data.len() / CELL_ID_LEN;
        let mut cell_ids = Vec::with_capacity(count);
        for chunk in cell_data.chunks_exact(CELL_ID_LEN) {
            cell_ids.push(crate::wire::be_u16(chunk, 0, "service_availability")?);
        }
        Ok(Self {
            reserved_future_use: flags & RESERVED_MASK,
            availability_flag,
            cell_ids,
        })
    }
}

impl Serialize for ServiceAvailabilityDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FLAGS_LEN + self.cell_ids.len() * CELL_ID_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = FLAGS_LEN + self.cell_ids.len() * CELL_ID_LEN;
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_availability_descriptor body exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, body_len)?;
        // reserved_future_use(7) preserved from parse (default all ones).
        let mut flags = self.reserved_future_use & RESERVED_MASK;
        if self.availability_flag {
            flags |= AVAILABILITY_FLAG_MASK;
        }
        buf[HEADER_LEN] = flags;
        let mut pos = HEADER_LEN + FLAGS_LEN;
        for cid in &self.cell_ids {
            buf[pos..pos + CELL_ID_LEN].copy_from_slice(&cid.to_be_bytes());
            pos += CELL_ID_LEN;
        }
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for ServiceAvailabilityDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SERVICE_AVAILABILITY";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_flag_and_cells() {
        // flags=0x80 (available), two cell_ids 0x0001 and 0x0002.
        let bytes = [TAG, 5, 0x80, 0x00, 0x01, 0x00, 0x02];
        let d = ServiceAvailabilityDescriptor::parse(&bytes).unwrap();
        assert!(d.availability_flag);
        assert_eq!(d.cell_ids, vec![0x0001, 0x0002]);
    }

    #[test]
    fn parse_flag_false() {
        let bytes = [TAG, 1, 0x00];
        let d = ServiceAvailabilityDescriptor::parse(&bytes).unwrap();
        assert!(!d.availability_flag);
        assert!(d.cell_ids.is_empty());
    }

    /// Every reserved_future_use(7) value re-serializes verbatim.
    #[test]
    fn reserved_preserved_for_every_value() {
        for r in 0u8..=0x7F {
            for flag in [0u8, 0x80] {
                let bytes = [TAG, 3, flag | r, 0x00, 0x01];
                let d = ServiceAvailabilityDescriptor::parse(&bytes).unwrap();
                assert_eq!(d.reserved_future_use, r);
                let mut buf = vec![0u8; d.serialized_len()];
                d.serialize_into(&mut buf).unwrap();
                assert_eq!(buf, bytes);
            }
        }
    }

    #[test]
    fn default_is_spec_value() {
        let d = ServiceAvailabilityDescriptor::new(false, vec![]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 1, 0x7F]);
    }

    #[test]
    fn parse_ignores_reserved_bits() {
        // reserved bits set, availability clear: 0x7F.
        let bytes = [TAG, 1, 0x7F];
        let d = ServiceAvailabilityDescriptor::parse(&bytes).unwrap();
        assert!(!d.availability_flag);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            ServiceAvailabilityDescriptor::parse(&[0x73, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x73, .. }
        ));
    }

    #[test]
    fn parse_rejects_zero_length_body() {
        assert!(matches!(
            ServiceAvailabilityDescriptor::parse(&[TAG, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_odd_cell_loop() {
        // body = flags + 1 odd byte.
        let bytes = [TAG, 2, 0x80, 0x00];
        assert!(matches!(
            ServiceAvailabilityDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_truncated_buffer() {
        let bytes = [TAG, 5, 0x80, 0x00, 0x01];
        assert!(matches!(
            ServiceAvailabilityDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = ServiceAvailabilityDescriptor::new(true, vec![0x00AB, 0x00CD, 0x00EF]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(ServiceAvailabilityDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = ServiceAvailabilityDescriptor::new(false, vec![0x0001]);
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_count() {
        // 128 cell_ids = 256 body bytes + flags = 257 > 255.
        let d = ServiceAvailabilityDescriptor::new(true, vec![0u16; 128]);
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = ServiceAvailabilityDescriptor::new(true, vec![0x0001, 0x0002]);
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
