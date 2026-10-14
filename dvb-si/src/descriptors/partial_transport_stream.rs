//! Partial Transport Stream Descriptor — ETSI EN 300 468 §7.2.1 (tag 0x63).
//!
//! Table 165 (PDF p. 155). Carried in the SIT (Selection Information Table) of a
//! partial TS to describe its bit-rate / smoothing-buffer characteristics. The
//! body is a FIXED 8-byte structure (NOT a loop): three reserved-prefixed
//! fields. The published markdown rendering of Table 165 is faithful to the PDF
//! (verified against PDF p. 155, §7.2.1):
//!
//! - reserved_future_use (2) + peak_rate (22)
//! - reserved_future_use (2) + minimum_overall_smoothing_rate (22)
//! - reserved_future_use (2) + maximum_overall_smoothing_buffer (14)
//!
//! The three `reserved_future_use`(2) regions are captured on parse and
//! re-emitted verbatim. Default value `11` = `reserved_future_use` = 1
//! (EN 300 468 V1.3.1-V1.20.1, audit §6.1: class A, stable in every edition;
//! `DVB_reserved_future_use` renamed to `reserved_future_use` in V1.17.1).

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for partial_transport_stream_descriptor.
pub const TAG: u8 = 0x63;
const HEADER_LEN: usize = 2;
/// Fixed payload length: 3×24-bit reserved-prefixed words (EN 300 468 Table 165).
const BODY_LEN: u8 = 8;
/// Maximum value for a 22-bit field (peak_rate / minimum_overall_smoothing_rate).
const MAX_22: u32 = (1 << 22) - 1;
/// Maximum value for the 14-bit maximum_overall_smoothing_buffer field.
const MAX_14: u16 = (1 << 14) - 1;
/// Default of each 2-bit `reserved_future_use` (EN 300 468 Table 165):
/// `reserved_future_use` = `1` (all editions, audit §6.1).
pub const RESERVED_FUTURE_USE_DEFAULT: u8 = 0b11;
const RESERVED_MASK: u8 = 0x03;

/// Partial Transport Stream Descriptor (tag 0x63).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PartialTransportStreamDescriptor {
    /// 22-bit peak_rate (188-byte packets per unit interval, PDF p. 155).
    pub peak_rate: u32,
    /// 22-bit minimum_overall_smoothing_rate (0x3FFFFF = undefined, PDF p. 155).
    pub minimum_overall_smoothing_rate: u32,
    /// 14-bit maximum_overall_smoothing_buffer (0x3FFF = undefined, PDF p. 155).
    pub maximum_overall_smoothing_buffer: u16,
    /// `reserved_future_use(2)` ahead of `peak_rate` (2 bits, right-aligned).
    /// Captured on parse, re-emitted verbatim; defaults to
    /// [`RESERVED_FUTURE_USE_DEFAULT`].
    pub reserved_future_use_peak_rate: u8,
    /// `reserved_future_use(2)` ahead of `minimum_overall_smoothing_rate`
    /// (2 bits, right-aligned). Captured on parse, re-emitted verbatim;
    /// defaults to [`RESERVED_FUTURE_USE_DEFAULT`].
    pub reserved_future_use_min_rate: u8,
    /// `reserved_future_use(2)` ahead of `maximum_overall_smoothing_buffer`
    /// (2 bits, right-aligned). Captured on parse, re-emitted verbatim;
    /// defaults to [`RESERVED_FUTURE_USE_DEFAULT`].
    pub reserved_future_use_buffer: u8,
}

impl PartialTransportStreamDescriptor {
    /// Build with the spec-default reserved bits.
    #[must_use]
    pub const fn new(
        peak_rate: u32,
        minimum_overall_smoothing_rate: u32,
        maximum_overall_smoothing_buffer: u16,
    ) -> Self {
        Self {
            peak_rate,
            minimum_overall_smoothing_rate,
            maximum_overall_smoothing_buffer,
            reserved_future_use_peak_rate: RESERVED_FUTURE_USE_DEFAULT,
            reserved_future_use_min_rate: RESERVED_FUTURE_USE_DEFAULT,
            reserved_future_use_buffer: RESERVED_FUTURE_USE_DEFAULT,
        }
    }
}

impl<'a> Parse<'a> for PartialTransportStreamDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "PartialTransportStreamDescriptor",
            "unexpected tag for partial_transport_stream_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "partial_transport_stream_descriptor length must equal 8",
            });
        }
        let peak_rate =
            (u32::from(body[0] & 0x3F) << 16) | (u32::from(body[1]) << 8) | u32::from(body[2]);
        let minimum_overall_smoothing_rate =
            (u32::from(body[3] & 0x3F) << 16) | (u32::from(body[4]) << 8) | u32::from(body[5]);
        let maximum_overall_smoothing_buffer =
            (u16::from(body[6] & 0x3F) << 8) | u16::from(body[7]);
        Ok(Self {
            peak_rate,
            minimum_overall_smoothing_rate,
            maximum_overall_smoothing_buffer,
            reserved_future_use_peak_rate: body[0] >> 6,
            reserved_future_use_min_rate: body[3] >> 6,
            reserved_future_use_buffer: body[6] >> 6,
        })
    }
}

impl Serialize for PartialTransportStreamDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN as usize
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.peak_rate > MAX_22 || self.minimum_overall_smoothing_rate > MAX_22 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "peak_rate / minimum_overall_smoothing_rate exceed 22 bits",
            });
        }
        if self.maximum_overall_smoothing_buffer > MAX_14 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "maximum_overall_smoothing_buffer exceeds 14 bits",
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
        buf[1] = BODY_LEN;
        let b = HEADER_LEN;
        // reserved_future_use bits re-emitted as captured (top 2 bits of each
        // high byte).
        buf[b] = (broadcast_common::len::fit_mask(
            self.reserved_future_use_peak_rate,
            RESERVED_MASK,
            "reserved_future_use_peak_rate",
        )? << 6)
            | ((self.peak_rate >> 16) as u8 & 0x3F);
        buf[b + 1] = (self.peak_rate >> 8) as u8;
        buf[b + 2] = self.peak_rate as u8;
        buf[b + 3] = (broadcast_common::len::fit_mask(
            self.reserved_future_use_min_rate,
            RESERVED_MASK,
            "reserved_future_use_min_rate",
        )? << 6)
            | ((self.minimum_overall_smoothing_rate >> 16) as u8 & 0x3F);
        buf[b + 4] = (self.minimum_overall_smoothing_rate >> 8) as u8;
        buf[b + 5] = self.minimum_overall_smoothing_rate as u8;
        buf[b + 6] = (broadcast_common::len::fit_mask(
            self.reserved_future_use_buffer,
            RESERVED_MASK,
            "reserved_future_use_buffer",
        )? << 6)
            | ((self.maximum_overall_smoothing_buffer >> 8) as u8 & 0x3F);
        buf[b + 7] = self.maximum_overall_smoothing_buffer as u8;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for PartialTransportStreamDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "PARTIAL_TRANSPORT_STREAM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields_and_captures_reserved() {
        // peak_rate=0x012345, min=0x0ABCDE, max=0x1234. Reserved bits set to 1.
        let bytes = [TAG, 8, 0xC1, 0x23, 0x45, 0xCA, 0xBC, 0xDE, 0xD2, 0x34];
        let d = PartialTransportStreamDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.peak_rate, 0x01_2345);
        assert_eq!(d.minimum_overall_smoothing_rate, 0x0A_BCDE);
        assert_eq!(d.maximum_overall_smoothing_buffer, 0x1234);
        assert_eq!(d.reserved_future_use_peak_rate, 0b11);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = PartialTransportStreamDescriptor::parse(&[0x64, 8, 0, 0, 0, 0, 0, 0, 0, 0])
            .unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x64, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = PartialTransportStreamDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        // length=8 but only 4 payload bytes present.
        let err = PartialTransportStreamDescriptor::parse(&[TAG, 8, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err =
            PartialTransportStreamDescriptor::parse(&[TAG, 7, 0, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = PartialTransportStreamDescriptor::new(0x0F_FFFF, 0x00_0001, 0x2ABC);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = PartialTransportStreamDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_emits_reserved_ones() {
        let d = PartialTransportStreamDescriptor::new(0, 0, 0);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // The three reserved-prefixed bytes have their top two bits set.
        assert_eq!(buf[2] & 0xC0, 0xC0);
        assert_eq!(buf[5] & 0xC0, 0xC0);
        assert_eq!(buf[8] & 0xC0, 0xC0);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = PartialTransportStreamDescriptor::new(1, 1, 1);
        let mut tiny = [0u8; 5];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_range_22bit() {
        let d = PartialTransportStreamDescriptor::new(1 << 22, 0, 0); // one past 22-bit max
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_over_range_14bit() {
        let d = PartialTransportStreamDescriptor::new(0, 0, 1 << 14); // one past 14-bit max
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = PartialTransportStreamDescriptor::new(0, 0, 0);
        assert_eq!(d.serialized_len() - 2, 8);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = PartialTransportStreamDescriptor::new(0x00_1234, 0x00_5678, 0x09AB);
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }

    /// EN 300 468 Table 165. All three `reserved_future_use(2)` regions are
    /// preserved verbatim (zero and non-default values).
    #[test]
    fn partial_transport_stream_reserved_preserved() {
        for r1 in 0u8..=3 {
            for r2 in 0u8..=3 {
                for r3 in 0u8..=3 {
                    let bytes = [
                        TAG,
                        8,
                        (r1 << 6) | 0x01,
                        0x23,
                        0x45,
                        (r2 << 6) | 0x0A,
                        0xBC,
                        0xDE,
                        (r3 << 6) | 0x12,
                        0x34,
                    ];
                    let d = PartialTransportStreamDescriptor::parse(&bytes).unwrap();
                    assert_eq!(
                        (
                            d.reserved_future_use_peak_rate,
                            d.reserved_future_use_min_rate,
                            d.reserved_future_use_buffer
                        ),
                        (r1, r2, r3)
                    );
                    let mut out = vec![0u8; d.serialized_len()];
                    d.serialize_into(&mut out).unwrap();
                    assert_eq!(out, bytes);
                }
            }
        }
    }

    #[test]
    fn partial_transport_stream_default_is_spec_value() {
        let d = PartialTransportStreamDescriptor::new(0x01_2345, 0x0A_BCDE, 0x1234);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out,
            [TAG, 8, 0xC1, 0x23, 0x45, 0xCA, 0xBC, 0xDE, 0xD2, 0x34]
        );
    }
}
