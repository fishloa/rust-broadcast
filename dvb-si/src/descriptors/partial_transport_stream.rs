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
//! Reserved bits are ignored on parse and emitted as 1s on serialize (matching
//! the `reserved_future_use` convention used elsewhere in this crate).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for partial_transport_stream_descriptor.
pub const TAG: u8 = 0x63;
const HEADER_LEN: usize = 2;
/// Fixed payload length: 3×24-bit reserved-prefixed words (EN 300 468 Table 165).
const BODY_LEN: u8 = 8;
/// Maximum value for a 22-bit field (peak_rate / minimum_overall_smoothing_rate).
const MAX_22: u32 = (1 << 22) - 1;
/// Maximum value for the 14-bit maximum_overall_smoothing_buffer field.
const MAX_14: u16 = (1 << 14) - 1;

/// Partial Transport Stream Descriptor (tag 0x63).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PartialTransportStreamDescriptor {
    /// 22-bit peak_rate (188-byte packets per unit interval, PDF p. 155).
    pub peak_rate: u32,
    /// 22-bit minimum_overall_smoothing_rate (0x3FFFFF = undefined, PDF p. 155).
    pub minimum_overall_smoothing_rate: u32,
    /// 14-bit maximum_overall_smoothing_buffer (0x3FFF = undefined, PDF p. 155).
    pub maximum_overall_smoothing_buffer: u16,
}

impl<'a> Parse<'a> for PartialTransportStreamDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "PartialTransportStreamDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for partial_transport_stream_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let total = HEADER_LEN + length;
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "PartialTransportStreamDescriptor body",
            });
        }
        if length != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "partial_transport_stream_descriptor length must equal 8",
            });
        }
        let b = HEADER_LEN;
        // reserved(2) + peak_rate(22): top 2 bits of byte 0 are reserved.
        let peak_rate = (u32::from(bytes[b] & 0x3F) << 16)
            | (u32::from(bytes[b + 1]) << 8)
            | u32::from(bytes[b + 2]);
        let minimum_overall_smoothing_rate = (u32::from(bytes[b + 3] & 0x3F) << 16)
            | (u32::from(bytes[b + 4]) << 8)
            | u32::from(bytes[b + 5]);
        // reserved(2) + maximum_overall_smoothing_buffer(14).
        let maximum_overall_smoothing_buffer =
            (u16::from(bytes[b + 6] & 0x3F) << 8) | u16::from(bytes[b + 7]);
        Ok(Self {
            peak_rate,
            minimum_overall_smoothing_rate,
            maximum_overall_smoothing_buffer,
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
        // reserved_future_use bits emitted as 1s (0xC0 mask on the high byte).
        buf[b] = 0xC0 | ((self.peak_rate >> 16) as u8 & 0x3F);
        buf[b + 1] = (self.peak_rate >> 8) as u8;
        buf[b + 2] = self.peak_rate as u8;
        buf[b + 3] = 0xC0 | ((self.minimum_overall_smoothing_rate >> 16) as u8 & 0x3F);
        buf[b + 4] = (self.minimum_overall_smoothing_rate >> 8) as u8;
        buf[b + 5] = self.minimum_overall_smoothing_rate as u8;
        buf[b + 6] = 0xC0 | ((self.maximum_overall_smoothing_buffer >> 8) as u8 & 0x3F);
        buf[b + 7] = self.maximum_overall_smoothing_buffer as u8;
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for PartialTransportStreamDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        BODY_LEN
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for PartialTransportStreamDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "PARTIAL_TRANSPORT_STREAM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields_and_ignores_reserved() {
        // peak_rate=0x012345, min=0x0ABCDE, max=0x1234. Reserved bits set to 1.
        let bytes = [TAG, 8, 0xC1, 0x23, 0x45, 0xCA, 0xBC, 0xDE, 0xD2, 0x34];
        let d = PartialTransportStreamDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.peak_rate, 0x01_2345);
        assert_eq!(d.minimum_overall_smoothing_rate, 0x0A_BCDE);
        assert_eq!(d.maximum_overall_smoothing_buffer, 0x1234);
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
        let d = PartialTransportStreamDescriptor {
            peak_rate: 0x0F_FFFF,
            minimum_overall_smoothing_rate: 0x00_0001,
            maximum_overall_smoothing_buffer: 0x2ABC,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = PartialTransportStreamDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_emits_reserved_ones() {
        let d = PartialTransportStreamDescriptor {
            peak_rate: 0,
            minimum_overall_smoothing_rate: 0,
            maximum_overall_smoothing_buffer: 0,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // The three reserved-prefixed bytes have their top two bits set.
        assert_eq!(buf[2] & 0xC0, 0xC0);
        assert_eq!(buf[5] & 0xC0, 0xC0);
        assert_eq!(buf[8] & 0xC0, 0xC0);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = PartialTransportStreamDescriptor {
            peak_rate: 1,
            minimum_overall_smoothing_rate: 1,
            maximum_overall_smoothing_buffer: 1,
        };
        let mut tiny = [0u8; 5];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_range_22bit() {
        let d = PartialTransportStreamDescriptor {
            peak_rate: 1 << 22, // one past 22-bit max
            minimum_overall_smoothing_rate: 0,
            maximum_overall_smoothing_buffer: 0,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_over_range_14bit() {
        let d = PartialTransportStreamDescriptor {
            peak_rate: 0,
            minimum_overall_smoothing_rate: 0,
            maximum_overall_smoothing_buffer: 1 << 14, // one past 14-bit max
        };
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = PartialTransportStreamDescriptor {
            peak_rate: 0,
            minimum_overall_smoothing_rate: 0,
            maximum_overall_smoothing_buffer: 0,
        };
        assert_eq!(d.descriptor_length(), 8);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = PartialTransportStreamDescriptor {
            peak_rate: 0x00_1234,
            minimum_overall_smoothing_rate: 0x00_5678,
            maximum_overall_smoothing_buffer: 0x09AB,
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
