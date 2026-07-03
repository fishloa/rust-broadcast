//! Multiplex Buffer Utilization Descriptor — ISO/IEC 13818-1 §2.6.22 (tag 0x0C).
//!
//! Bound on the portion of the multiplex buffer occupied without
//! overflow/underflow of the STD.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for multiplex_buffer_utilization_descriptor.
pub const TAG: u8 = 0x0C;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 4;

/// Multiplex Buffer Utilization Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct MultiplexBufferUtilizationDescriptor {
    /// Bound valid flag.
    pub bound_valid_flag: bool,
    /// LTW offset lower bound (15 bits).
    pub ltw_offset_lower_bound: u16,
    /// LTW offset upper bound (15 bits).
    pub ltw_offset_upper_bound: u16,
}

impl<'a> Parse<'a> for MultiplexBufferUtilizationDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MultiplexBufferUtilizationDescriptor",
            "unexpected tag for multiplex_buffer_utilization_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "multiplex_buffer_utilization_descriptor length must equal 4",
            });
        }
        let bound_valid_flag = (body[0] & 0x80) != 0;
        let ltw_offset_lower_bound = ((u16::from(body[0]) & 0x7F) << 8) | u16::from(body[1]);
        let ltw_offset_upper_bound = ((u16::from(body[2]) & 0x7F) << 8) | u16::from(body[3]);
        Ok(Self {
            bound_valid_flag,
            ltw_offset_lower_bound,
            ltw_offset_upper_bound,
        })
    }
}

impl Serialize for MultiplexBufferUtilizationDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN] = ((self.bound_valid_flag as u8) << 7)
            | ((self.ltw_offset_lower_bound >> 8) as u8 & 0x7F);
        buf[HEADER_LEN + 1] = (self.ltw_offset_lower_bound & 0xFF) as u8;
        buf[HEADER_LEN + 2] = (self.ltw_offset_upper_bound >> 8) as u8 & 0x7F;
        buf[HEADER_LEN + 3] = (self.ltw_offset_upper_bound & 0xFF) as u8;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for MultiplexBufferUtilizationDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MULTIPLEX_BUFFER_UTILIZATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            4,
            0b1_1111111,
            0xAB, // bound_valid=1, lower_bound=0x7FAB
            0b1_1111111,
            0xCD, // reserved(1) in source, upper_bound=0x7FCD
        ];
        let d = MultiplexBufferUtilizationDescriptor::parse(&bytes).unwrap();
        assert!(d.bound_valid_flag);
        assert_eq!(d.ltw_offset_lower_bound, 0x7FAB);
        assert_eq!(d.ltw_offset_upper_bound, 0x7FCD);
    }

    #[test]
    fn serialize_round_trip() {
        let d = MultiplexBufferUtilizationDescriptor {
            bound_valid_flag: false,
            ltw_offset_lower_bound: 0x1234,
            ltw_offset_upper_bound: 0x5678,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MultiplexBufferUtilizationDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MultiplexBufferUtilizationDescriptor::parse(&[0x0D, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x0D, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = MultiplexBufferUtilizationDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = MultiplexBufferUtilizationDescriptor {
            bound_valid_flag: false,
            ltw_offset_lower_bound: 0,
            ltw_offset_upper_bound: 0,
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
