//! Smoothing Buffer Descriptor — ISO/IEC 13818-1 §2.6.30 (tag 0x10).
//!
//! Describes the leak rate and size of a smoothing buffer.
//! `sb_leak_rate` is in units of 400 bits/s; `sb_size` in units of
//! 1 byte (§2.6.31).

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for smoothing_buffer_descriptor.
pub const TAG: u8 = 0x10;
const HEADER_LEN: usize = 2;
/// Default of each 2-bit `reserved` ahead of the 22-bit fields (H.222.0 Table
/// 2-69): clause 2.1 "reserved" = `1` (all 9 editions 07/1995-08/2023, audit
/// §5; confirmed by the TSDuck oracle PMT).
pub const RESERVED_DEFAULT: u8 = 0b11;
const RESERVED_MASK: u8 = 0x03;
const BODY_LEN: u8 = 6;

/// Smoothing Buffer Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct SmoothingBufferDescriptor {
    /// SB leak rate in units of 400 bits/s (22 bits).
    pub sb_leak_rate: u32,
    /// SB size in bytes (22 bits).
    pub sb_size: u32,
    /// `reserved(2)` ahead of `sb_leak_rate` (2 bits, right-aligned).
    /// Captured on parse, re-emitted verbatim; defaults to [`RESERVED_DEFAULT`].
    pub reserved_leak_rate: u8,
    /// `reserved(2)` ahead of `sb_size` (2 bits, right-aligned). Captured on
    /// parse, re-emitted verbatim; defaults to [`RESERVED_DEFAULT`].
    pub reserved_size: u8,
}

impl SmoothingBufferDescriptor {
    /// Build with the spec-default reserved bits.
    #[must_use]
    pub const fn new(sb_leak_rate: u32, sb_size: u32) -> Self {
        Self {
            sb_leak_rate,
            sb_size,
            reserved_leak_rate: RESERVED_DEFAULT,
            reserved_size: RESERVED_DEFAULT,
        }
    }
}

impl<'a> Parse<'a> for SmoothingBufferDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SmoothingBufferDescriptor",
            "unexpected tag for smoothing_buffer_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "smoothing_buffer_descriptor length must equal 6",
            });
        }
        let sb_leak_rate =
            ((u32::from(body[0]) & 0x3F) << 16) | (u32::from(body[1]) << 8) | u32::from(body[2]);
        let sb_size =
            ((u32::from(body[3]) & 0x3F) << 16) | (u32::from(body[4]) << 8) | u32::from(body[5]);
        Ok(Self {
            sb_leak_rate,
            sb_size,
            reserved_leak_rate: body[0] >> 6,
            reserved_size: body[3] >> 6,
        })
    }
}

impl Serialize for SmoothingBufferDescriptor {
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
        // 22-bit fields: a wider value used to be masked down silently
        // (audit r03-W3 class).
        let slr = broadcast_common::len::fit_bits(u64::from(self.sb_leak_rate), 22, "sb_leak_rate")? as u32;
        let ss = broadcast_common::len::fit_bits(u64::from(self.sb_size), 22, "sb_size")? as u32;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN] =
            ((self.reserved_leak_rate & RESERVED_MASK) << 6) | ((slr >> 16) & 0x3F) as u8;
        buf[HEADER_LEN + 1] = ((slr >> 8) & 0xFF) as u8;
        buf[HEADER_LEN + 2] = (slr & 0xFF) as u8;
        buf[HEADER_LEN + 3] =
            ((self.reserved_size & RESERVED_MASK) << 6) | ((ss >> 16) & 0x3F) as u8;
        buf[HEADER_LEN + 4] = ((ss >> 8) & 0xFF) as u8;
        buf[HEADER_LEN + 5] = (ss & 0xFF) as u8;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for SmoothingBufferDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SMOOTHING_BUFFER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            6,
            0b00_111111,
            0xAB,
            0xCD, // sb_leak_rate=0x3FABCD
            0b00_000010,
            0x34,
            0x56, // sb_size=0x023456
        ];
        let d = SmoothingBufferDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.sb_leak_rate, 0x3FABCD);
        assert_eq!(d.sb_size, 0x023456);
    }

    #[test]
    fn serialize_round_trip() {
        let d = SmoothingBufferDescriptor::new(0x1A2B3C, 0x2D4E5F);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = SmoothingBufferDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = SmoothingBufferDescriptor::parse(&[0x11, 6, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x11, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = SmoothingBufferDescriptor::parse(&[TAG, 5, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = SmoothingBufferDescriptor::new(0, 0);
        let mut tiny = vec![0u8; 4];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    /// H.222.0 Table 2-69. Parsed reserved bits are preserved (zero-reserved
    /// senders are no longer normalized to `11`); defaults carry the spec fill.
    #[test]
    fn smoothing_buffer_reserved_preserved() {
        for r1 in 0u8..=3 {
            for r2 in 0u8..=3 {
                let bytes = [
                    TAG,
                    6,
                    (r1 << 6) | 0x3F,
                    0xAB,
                    0xCD,
                    (r2 << 6) | 0x02,
                    0x34,
                    0x56,
                ];
                let d = SmoothingBufferDescriptor::parse(&bytes).unwrap();
                assert_eq!((d.reserved_leak_rate, d.reserved_size), (r1, r2));
                let mut out = vec![0u8; d.serialized_len()];
                d.serialize_into(&mut out).unwrap();
                assert_eq!(out, bytes);
            }
        }
    }

    #[test]
    fn smoothing_buffer_default_is_spec_value() {
        let d = SmoothingBufferDescriptor::new(0x3FABCD, 0x023456);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 6, 0xFF, 0xAB, 0xCD, 0xC2, 0x34, 0x56]);
    }
}
