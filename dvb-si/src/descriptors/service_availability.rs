//! Service Availability Descriptor — ETSI EN 300 468 §6.2.36 (tag 0x72, Table 90, PDF p. 101).
//!
//! Carried inside the SDT. Body layout (Table 90):
//! `availability_flag` (1 bit) + `reserved_future_use` (7 bits), then a
//! `for (i=0;i<N;i++) { cell_id (16) }` loop listing the cells in which the
//! service is (un)available. `availability_flag`=1 → service available in the
//! listed cells; =0 → unavailable in the listed cells.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

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

/// Service Availability Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceAvailabilityDescriptor {
    /// `availability_flag`: true = available in listed cells, false = unavailable.
    pub availability_flag: bool,
    /// cell_id entries in wire order.
    pub cell_ids: Vec<u16>,
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
        if (body.len() - FLAGS_LEN) % CELL_ID_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_availability cell_id loop must be a multiple of 2 bytes",
            });
        }
        let flags = body[0];
        // reserved_future_use (7 bits) ignored on parse (§5.1).
        let availability_flag = flags & AVAILABILITY_FLAG_MASK != 0;
        let count = (body.len() - FLAGS_LEN) / CELL_ID_LEN;
        let mut cell_ids = Vec::with_capacity(count);
        let mut pos = FLAGS_LEN;
        for _ in 0..count {
            cell_ids.push(u16::from_be_bytes([body[pos], body[pos + 1]]));
            pos += CELL_ID_LEN;
        }
        Ok(Self {
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
        buf[0] = TAG;
        buf[1] = body_len as u8;
        // reserved 7 bits emitted as 1s (§5.1).
        let mut flags = RESERVED_MASK;
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
impl<'a> crate::traits::DescriptorDef<'a> for ServiceAvailabilityDescriptor {
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
        let d = ServiceAvailabilityDescriptor {
            availability_flag: true,
            cell_ids: vec![0x00AB, 0x00CD, 0x00EF],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(ServiceAvailabilityDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = ServiceAvailabilityDescriptor {
            availability_flag: false,
            cell_ids: vec![0x0001],
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_count() {
        // 128 cell_ids = 256 body bytes + flags = 257 > 255.
        let d = ServiceAvailabilityDescriptor {
            availability_flag: true,
            cell_ids: vec![0u16; 128],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = ServiceAvailabilityDescriptor {
            availability_flag: true,
            cell_ids: vec![0x0001, 0x0002],
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
