//! IP Signalling Descriptor — ETSI TS 102 727 V1.1.1 §10.8.2, Table 81
//! (AIT tag 0x11). PDF p.169, transcribed in
//! `docs/descriptors/ts_102_727/81-*.md`, verified against the rendered page.
//!
//! Defined for the "common" or "application" AIT loop. Fixed 3-byte body: the
//! 24-bit `platform_id` of the organisation providing IP/MAC streams (the
//! key of the INT with `action_type` 0x01, EN 301 192).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for ip_signalling_descriptor (AIT namespace).
pub const TAG: u8 = 0x11;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 3;
const PLATFORM_ID_MASK: u32 = 0x00FF_FFFF;

/// IP Signalling Descriptor (AIT tag 0x11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct IpSignallingDescriptor {
    /// `platform_id`(24) — allocations are in TR 101 162.
    pub platform_id: u32,
}

impl<'a> Parse<'a> for IpSignallingDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "IpSignallingDescriptor",
            "unexpected tag for ip_signalling_descriptor",
        )?;
        if body.len() != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "ip_signalling_descriptor body must be 3 bytes",
            });
        }
        Ok(Self {
            platform_id: u32::from_be_bytes([0, body[0], body[1], body[2]]),
        })
    }
}

impl Serialize for IpSignallingDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let id =
            broadcast_common::len::fit_mask(self.platform_id, PLATFORM_ID_MASK, "platform_id")?;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, BODY_LEN)?;
        buf[HEADER_LEN..len].copy_from_slice(&id.to_be_bytes()[1..]);
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for IpSignallingDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "IP_SIGNALLING";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let bytes = [TAG, 3, 0x00, 0x00, 0x2A];
        let d = IpSignallingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.platform_id, 0x2A);
        let mut out = [0u8; 5];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn round_trip_max_platform_id() {
        let bytes = [TAG, 3, 0xFF, 0xFF, 0xFF];
        let d = IpSignallingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.platform_id, 0x00FF_FFFF);
        let mut out = [0u8; 5];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn wrong_length_and_over_range_rejected() {
        assert!(IpSignallingDescriptor::parse(&[TAG, 2, 0, 0]).is_err());
        assert!(IpSignallingDescriptor::parse(&[TAG, 4, 0, 0, 0, 0]).is_err());
        let d = IpSignallingDescriptor {
            platform_id: 0x0100_0000,
        };
        assert!(d.serialize_into(&mut [0u8; 5]).is_err());
    }
}
