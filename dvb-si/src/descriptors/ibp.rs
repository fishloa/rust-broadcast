//! IBP Descriptor — ISO/IEC 13818-1 §2.6.34 (tag 0x12).
//!
//! Describes the GOP structure of the associated video elementary stream.
//! `max_gop_length` value 0 is forbidden (§2.6.35).

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for ibp_descriptor.
pub const TAG: u8 = 0x12;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 2;

/// IBP Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct IbpDescriptor {
    /// Closed GOP flag.
    pub closed_gop_flag: bool,
    /// Identical GOP flag.
    pub identical_gop_flag: bool,
    /// Maximum GOP length in pictures (14 bits). Value 0 is forbidden.
    pub max_gop_length: u16,
}

impl<'a> Parse<'a> for IbpDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "IbpDescriptor",
            "unexpected tag for ibp_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "ibp_descriptor length must equal 2",
            });
        }
        let closed_gop_flag = (body[0] & 0x80) != 0;
        let identical_gop_flag = (body[0] & 0x40) != 0;
        let max_gop_length = ((u16::from(body[0]) & 0x3F) << 8) | u16::from(body[1]);
        Ok(Self {
            closed_gop_flag,
            identical_gop_flag,
            max_gop_length,
        })
    }
}

impl Serialize for IbpDescriptor {
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
        buf[HEADER_LEN] = ((self.closed_gop_flag as u8) << 7)
            | ((self.identical_gop_flag as u8) << 6)
            | ((self.max_gop_length >> 8) as u8 & 0x3F);
        buf[HEADER_LEN + 1] = (self.max_gop_length & 0xFF) as u8;
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for IbpDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "IBP";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            2,
            0b1111_1111,
            0xAB, // closed=1, identical=1, max_gop=0x3FAB
        ];
        let d = IbpDescriptor::parse(&bytes).unwrap();
        assert!(d.closed_gop_flag);
        assert!(d.identical_gop_flag);
        assert_eq!(d.max_gop_length, 0x3FAB);
    }

    #[test]
    fn serialize_round_trip() {
        let d = IbpDescriptor {
            closed_gop_flag: false,
            identical_gop_flag: true,
            max_gop_length: 12,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = IbpDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn max_gop_allowed_max() {
        let d = IbpDescriptor {
            closed_gop_flag: false,
            identical_gop_flag: false,
            max_gop_length: 0x3FFF,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = IbpDescriptor::parse(&buf).unwrap();
        assert_eq!(reparsed.max_gop_length, 0x3FFF);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = IbpDescriptor::parse(&[0x13, 2, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x13, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = IbpDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = IbpDescriptor {
            closed_gop_flag: false,
            identical_gop_flag: false,
            max_gop_length: 1,
        };
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
