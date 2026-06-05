//! Short Smoothing Buffer Descriptor — ETSI EN 300 468 §6.2.38 (tag 0x61).
//!
//! Table 94 (PDF p. 103). A 2-bit sb_size + 6-bit sb_leak_rate packed into the
//! first payload byte (Table 95 / Table 96 coding), followed by an arbitrary
//! number of `DVB_reserved` bytes carried raw.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for short_smoothing_buffer_descriptor.
pub const TAG: u8 = 0x61;
const HEADER_LEN: usize = 2;
/// Minimum payload: the packed sb_size/sb_leak_rate byte (EN 300 468 Table 94).
const FIXED_LEN: usize = 1;

/// Short Smoothing Buffer Descriptor (tag 0x61).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ShortSmoothingBufferDescriptor<'a> {
    /// 2-bit sb_size (ETSI Table 95, PDF p. 103): 1 = 1 536 bytes, others reserved.
    pub sb_size: u8,
    /// 6-bit sb_leak_rate (ETSI Table 96, PDF p. 104).
    pub sb_leak_rate: u8,
    /// Raw DVB_reserved tail bytes; preserved verbatim for round-trips.
    pub dvb_reserved: &'a [u8],
}

impl<'a> Parse<'a> for ShortSmoothingBufferDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "ShortSmoothingBufferDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for short_smoothing_buffer_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "ShortSmoothingBufferDescriptor body",
            });
        }
        if length < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "short_smoothing_buffer_descriptor body shorter than 1 byte",
            });
        }
        let packed = bytes[HEADER_LEN];
        let sb_size = packed >> 6; // 2-bit field
        let sb_leak_rate = packed & 0x3F; // 6-bit field
        let dvb_reserved = &bytes[HEADER_LEN + FIXED_LEN..end];
        Ok(Self {
            sb_size,
            sb_leak_rate,
            dvb_reserved,
        })
    }
}

impl Serialize for ShortSmoothingBufferDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.dvb_reserved.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let body = FIXED_LEN + self.dvb_reserved.len();
        if body > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "short_smoothing_buffer_descriptor body exceeds 255 bytes",
            });
        }
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body as u8;
        buf[HEADER_LEN] = ((self.sb_size & 0x03) << 6) | (self.sb_leak_rate & 0x3F);
        let tail_start = HEADER_LEN + FIXED_LEN;
        buf[tail_start..tail_start + self.dvb_reserved.len()].copy_from_slice(self.dvb_reserved);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for ShortSmoothingBufferDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (FIXED_LEN + self.dvb_reserved.len()) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ShortSmoothingBufferDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SHORT_SMOOTHING_BUFFER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_packed_fields() {
        // sb_size=1 (0b01), sb_leak_rate=0x12 → packed = 0b01_010010 = 0x52.
        let bytes = [TAG, 1, 0x52];
        let d = ShortSmoothingBufferDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.sb_size, 1);
        assert_eq!(d.sb_leak_rate, 0x12);
        assert!(d.dvb_reserved.is_empty());
    }

    #[test]
    fn parse_preserves_reserved_tail() {
        let bytes = [TAG, 3, 0x52, 0xAA, 0xBB];
        let d = ShortSmoothingBufferDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.dvb_reserved, &[0xAA, 0xBB]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = ShortSmoothingBufferDescriptor::parse(&[0x62, 1, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x62, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = ShortSmoothingBufferDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_empty_body() {
        // length=0: no packed byte.
        let err = ShortSmoothingBufferDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_length_overrun() {
        // length=3 but only the packed byte present.
        let err = ShortSmoothingBufferDescriptor::parse(&[TAG, 3, 0x52]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = ShortSmoothingBufferDescriptor {
            sb_size: 1,
            sb_leak_rate: 0x2A,
            dvb_reserved: &[0x01, 0x02, 0x03],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ShortSmoothingBufferDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = ShortSmoothingBufferDescriptor {
            sb_size: 1,
            sb_leak_rate: 1,
            dvb_reserved: &[],
        };
        let mut tiny = [0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        let tail = vec![0u8; 255]; // 1 packed + 255 = 256 > 255
        let d = ShortSmoothingBufferDescriptor {
            sb_size: 1,
            sb_leak_rate: 1,
            dvb_reserved: &tail,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = ShortSmoothingBufferDescriptor {
            sb_size: 1,
            sb_leak_rate: 0,
            dvb_reserved: &[0xFF, 0xFF],
        };
        assert_eq!(d.descriptor_length(), 3);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_is_stable() {
        // Borrowed `&[u8]` cannot be deserialized from a JSON array by
        // serde_json; matching the borrowed-bytes descriptors in this crate we
        // exercise the serialize path and assert it is deterministic.
        let d = ShortSmoothingBufferDescriptor {
            sb_size: 1,
            sb_leak_rate: 0x3F,
            dvb_reserved: &[0xCA, 0xFE],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, serde_json::to_string(&d.clone()).unwrap());
    }
}
