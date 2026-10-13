//! Time Slice and FEC Identifier Descriptor — ETSI EN 301 192 §9.5 Table 40 /
//! ETSI TS 102 772 §6.2 Table 4 (tag 0x77).
//!
//! Carried in the INT / PMT to describe DVB-H time-slicing and MPE-FEC for an
//! elementary stream. Per en_301_192.md "Table 40" (PDF p. 53) and
//! ts_102_772_mpe_ifec.md "Table 4 — Time Slice and FEC identifier descriptor"
//! (PDF p. 19) the fixed 3-byte head is:
//!   byte0: time_slicing(1) + mpe_fec(2) + reserved_for_future_use(2) + frame_size(3)
//!   byte1: max_burst_duration(8)
//!   byte2: max_average_rate(4) + time_slice_fec_id(4)
//! followed by `id_selector_byte` for the remainder of the descriptor.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for time_slice_fec_identifier_descriptor.
pub const TAG: u8 = 0x77;
const HEADER_LEN: usize = 2;
const FIXED_LEN: usize = 3;

/// Shift of `reserved_for_future_use(2)` in byte 0 (bits 4..3).
const RESERVED_SHIFT: u8 = 3;
/// Width mask of the 2-bit `reserved_for_future_use`.
const RESERVED_MASK: u8 = 0x03;
/// Spec default of `reserved_for_future_use(2)`: `11` (EN 301 192 V1.4.1-V1.8.1
/// Table 40, TS 102 772 Table 4; audit §6.3 class A).
pub const RESERVED_FOR_FUTURE_USE_DEFAULT: u8 = 0b11;

/// Largest representable 2-bit mpe_fec.
const MPE_FEC_MAX: u8 = 0x03;
/// Largest representable 3-bit frame_size.
const FRAME_SIZE_MAX: u8 = 0x07;
/// Largest representable 4-bit max_average_rate.
const MAX_AVERAGE_RATE_MAX: u8 = 0x0F;
/// Largest representable 4-bit time_slice_fec_id.
const TIME_SLICE_FEC_ID_MAX: u8 = 0x0F;

/// Time Slice and FEC Identifier Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct TimeSliceFecIdentifierDescriptor<'a> {
    /// 1-bit time_slicing flag (1 = time slicing used).
    pub time_slicing: bool,
    /// 2-bit mpe_fec algorithm selector (Table 41: 0=none, 1=RS(255,191,64)).
    pub mpe_fec: u8,
    /// `reserved_for_future_use(2)` (byte 0 bits 4..3, right-aligned).
    /// Captured on parse and re-emitted verbatim;
    /// [`TimeSliceFecIdentifierDescriptor::new`] defaults to
    /// [`RESERVED_FOR_FUTURE_USE_DEFAULT`] (`0b11`).
    pub reserved_for_future_use: u8,
    /// 3-bit frame_size code (Table 42).
    pub frame_size: u8,
    /// 8-bit max_burst_duration.
    pub max_burst_duration: u8,
    /// 4-bit max_average_rate code (Table 43).
    pub max_average_rate: u8,
    /// 4-bit time_slice_fec_id.
    pub time_slice_fec_id: u8,
    /// Trailing id_selector bytes.
    /// Kept raw deliberately; interpretation is registry-dependent per
    /// EN 301 192 / TS 102 772.
    pub id_selector: &'a [u8],
}

impl<'a> TimeSliceFecIdentifierDescriptor<'a> {
    /// Build a descriptor with the spec-default reserved bits.
    #[must_use]
    pub const fn new(
        time_slicing: bool,
        mpe_fec: u8,
        frame_size: u8,
        max_burst_duration: u8,
        max_average_rate: u8,
        time_slice_fec_id: u8,
        id_selector: &'a [u8],
    ) -> Self {
        Self {
            time_slicing,
            mpe_fec,
            reserved_for_future_use: RESERVED_FOR_FUTURE_USE_DEFAULT,
            frame_size,
            max_burst_duration,
            max_average_rate,
            time_slice_fec_id,
            id_selector,
        }
    }
}

impl<'a> Parse<'a> for TimeSliceFecIdentifierDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "TimeSliceFecIdentifierDescriptor",
            "unexpected tag for time_slice_fec_identifier_descriptor",
        )?;
        if body.len() < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "time_slice_fec_identifier_descriptor body shorter than 3 bytes",
            });
        }
        let b0 = body[0];
        let time_slicing = (b0 & 0x80) != 0;
        let mpe_fec = (b0 >> 5) & MPE_FEC_MAX;
        let reserved_for_future_use = (b0 >> RESERVED_SHIFT) & RESERVED_MASK;
        let frame_size = b0 & FRAME_SIZE_MAX;
        let max_burst_duration = body[1];
        let max_average_rate = (body[2] >> 4) & MAX_AVERAGE_RATE_MAX;
        let time_slice_fec_id = body[2] & TIME_SLICE_FEC_ID_MAX;
        let id_selector = &body[FIXED_LEN..];
        Ok(Self {
            time_slicing,
            mpe_fec,
            reserved_for_future_use,
            frame_size,
            max_burst_duration,
            max_average_rate,
            time_slice_fec_id,
            id_selector,
        })
    }
}

impl Serialize for TimeSliceFecIdentifierDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.id_selector.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.mpe_fec > MPE_FEC_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "mpe_fec exceeds 2 bits",
            });
        }
        if self.frame_size > FRAME_SIZE_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "frame_size exceeds 3 bits",
            });
        }
        if self.max_average_rate > MAX_AVERAGE_RATE_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "max_average_rate exceeds 4 bits",
            });
        }
        if self.time_slice_fec_id > TIME_SLICE_FEC_ID_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "time_slice_fec_id exceeds 4 bits",
            });
        }
        if FIXED_LEN + self.id_selector.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "time_slice_fec_identifier_descriptor body exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, FIXED_LEN + self.id_selector.len())?;
        buf[2] = (u8::from(self.time_slicing) << 7)
            | ((self.mpe_fec & MPE_FEC_MAX) << 5)
            | ((self.reserved_for_future_use & RESERVED_MASK) << RESERVED_SHIFT)
            | (self.frame_size & FRAME_SIZE_MAX);
        buf[3] = self.max_burst_duration;
        buf[4] = ((self.max_average_rate & MAX_AVERAGE_RATE_MAX) << 4)
            | (self.time_slice_fec_id & TIME_SLICE_FEC_ID_MAX);
        buf[HEADER_LEN + FIXED_LEN..len].copy_from_slice(self.id_selector);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for TimeSliceFecIdentifierDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TIME_SLICE_FEC_IDENTIFIER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_id_selector() {
        // time_slicing=1, mpe_fec=1, frame_size=3, reserved=11, burst=0x2A,
        // max_average_rate=5, time_slice_fec_id=0xA.
        let b0 = 0x80 | (1 << 5) | 0x18 | 0x03;
        let bytes = [TAG, 3, b0, 0x2A, (5 << 4) | 0x0A];
        let d = TimeSliceFecIdentifierDescriptor::parse(&bytes).unwrap();
        assert!(d.time_slicing);
        assert_eq!(d.mpe_fec, 1);
        assert_eq!(d.frame_size, 3);
        assert_eq!(d.max_burst_duration, 0x2A);
        assert_eq!(d.max_average_rate, 5);
        assert_eq!(d.time_slice_fec_id, 0xA);
        assert!(d.id_selector.is_empty());
    }

    #[test]
    fn parse_with_id_selector() {
        let bytes = [TAG, 5, 0x00, 0x00, 0x00, 0xAA, 0xBB];
        let d = TimeSliceFecIdentifierDescriptor::parse(&bytes).unwrap();
        assert!(!d.time_slicing);
        assert_eq!(d.id_selector, &[0xAA, 0xBB]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            TimeSliceFecIdentifierDescriptor::parse(&[0x78, 3, 0, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x78, .. }
        ));
    }

    #[test]
    fn parse_rejects_body_too_short() {
        let bytes = [TAG, 2, 0, 0];
        assert!(matches!(
            TimeSliceFecIdentifierDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let bytes = [TAG, 5, 0, 0, 0];
        assert!(matches!(
            TimeSliceFecIdentifierDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = TimeSliceFecIdentifierDescriptor::new(true, 1, 2, 0x10, 6, 0, &[0x01, 0x02, 0x03]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(TimeSliceFecIdentifierDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_frame_size_over_range() {
        let d = TimeSliceFecIdentifierDescriptor::new(false, 0, 0x08, 0, 0, 0, &[]);
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serializes_to_stable_json() {
        // Borrowed `&[u8]` cannot deserialize from a JSON number array, so we
        // assert the Serialize impl is wired and emits stable JSON.
        let d = TimeSliceFecIdentifierDescriptor::new(true, 1, 3, 0x44, 7, 0xF, &[0xDE, 0xAD]);
        let j = serde_json::to_string(&d).unwrap();
        // Valid, re-parseable JSON (key order is map-defined, so we do not
        // assert byte-for-byte string stability).
        let _v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert!(j.contains("time_slice_fec_id"));
    }

    /// Every `reserved_for_future_use(2)` value re-serializes byte-identically.
    #[test]
    fn reserved_preserved() {
        for r in 0u8..=3 {
            let b0 = 0x80 | (1 << 5) | (r << 3) | 0x03;
            let bytes = [TAG, 5, b0, 0x2A, (5 << 4) | 0x0A, 0xAA, 0xBB];
            let d = TimeSliceFecIdentifierDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved_for_future_use, r);
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes, "r {r}");
        }
    }

    /// `new()` serializes the spec value `11`.
    #[test]
    fn default_reserved_is_spec_value() {
        let d = TimeSliceFecIdentifierDescriptor::new(false, 0, 0, 0, 0, 0, &[]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[2], 0x18);
    }
}
