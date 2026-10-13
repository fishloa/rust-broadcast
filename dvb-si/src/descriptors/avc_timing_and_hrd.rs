//! AVC Timing and HRD Descriptor — ISO/IEC 13818-1 §2.6.66 (tag 0x2A).
//!
//! Provides encoding-parameters timestamps, HRD management validity,
//! and frame-rate conversion flags for an AVC/H.264 elementary stream.
//! The picture_and_timing_info block is conditional on
//! `picture_and_timing_info_present`; the inner N/K fields within it are
//! conditional on `90kHz_flag == 0`.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for AVC_timing_and_HRD_descriptor.
pub const TAG: u8 = 0x2A;
const HEADER_LEN: usize = 2;
/// Default of the 6-bit `reserved` in byte 0 (H.222.0 Table 2-93): clause 2.1
/// "reserved" = `1` (all editions 06/2012-08/2023, audit §5; confirmed by the
/// TSDuck oracle PMT).
pub const RESERVED_6_DEFAULT: u8 = 0b11_1111;
/// Default of the 7-bit `reserved` after `90kHz_flag` (same rule).
pub const RESERVED_7_DEFAULT: u8 = 0x7F;
/// Default of the trailing 5-bit `reserved` after the frame-rate flags (same
/// rule).
pub const RESERVED_5_DEFAULT: u8 = 0b1_1111;
const RESERVED_6_MASK: u8 = 0x3F;
const RESERVED_7_MASK: u8 = 0x7F;
const RESERVED_5_MASK: u8 = 0x1F;
const FIXED_BODY_LEN: u8 = 1; // first byte always present (hrd_management_valid_flag..picture_and_timing_info_present)

/// Picture and timing info block — present when `picture_and_timing_info_present` is true.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AvcPictureTiming {
    /// 1 — 90 kHz clock; 0 — N/K clock used.
    pub _90khz_flag: bool,
    /// N parameter (when `_90khz_flag` is false).
    pub n: Option<u32>,
    /// K parameter (when `_90khz_flag` is false).
    pub k: Option<u32>,
    /// num_units_in_tick.
    pub num_units_in_tick: u32,
    /// `reserved(7)` after `90kHz_flag` (7 bits, right-aligned). Captured on
    /// parse, re-emitted verbatim; [`AvcPictureTiming::new`] uses
    /// [`RESERVED_7_DEFAULT`].
    pub reserved_7: u8,
}

impl AvcPictureTiming {
    /// Build with the spec-default reserved bits.
    #[must_use]
    pub const fn new(
        _90khz_flag: bool,
        n: Option<u32>,
        k: Option<u32>,
        num_units_in_tick: u32,
    ) -> Self {
        Self {
            _90khz_flag,
            n,
            k,
            num_units_in_tick,
            reserved_7: RESERVED_7_DEFAULT,
        }
    }
}

/// AVC Timing and HRD Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct AvcTimingAndHrdDescriptor {
    /// HRD management valid flag.
    pub hrd_management_valid_flag: bool,
    /// Picture and timing info block, when present.
    pub picture_timing: Option<AvcPictureTiming>,
    /// Fixed frame rate flag.
    pub fixed_frame_rate_flag: bool,
    /// Temporal POC flag.
    pub temporal_poc_flag: bool,
    /// Picture to display conversion flag.
    pub picture_to_display_conversion_flag: bool,
    /// `reserved(6)` in byte 0 (6 bits, right-aligned). Captured on parse,
    /// re-emitted verbatim; [`AvcTimingAndHrdDescriptor::new`] uses
    /// [`RESERVED_6_DEFAULT`].
    pub reserved_6: u8,
    /// Trailing `reserved(5)` after the flags (5 bits, right-aligned).
    /// Captured on parse, re-emitted verbatim; defaults to
    /// [`RESERVED_5_DEFAULT`].
    pub reserved_5: u8,
}

impl AvcTimingAndHrdDescriptor {
    /// Build with the spec-default reserved bits.
    #[must_use]
    pub const fn new(
        hrd_management_valid_flag: bool,
        picture_timing: Option<AvcPictureTiming>,
        fixed_frame_rate_flag: bool,
        temporal_poc_flag: bool,
        picture_to_display_conversion_flag: bool,
    ) -> Self {
        Self {
            hrd_management_valid_flag,
            picture_timing,
            fixed_frame_rate_flag,
            temporal_poc_flag,
            picture_to_display_conversion_flag,
            reserved_6: RESERVED_6_DEFAULT,
            reserved_5: RESERVED_5_DEFAULT,
        }
    }
}

impl<'a> Parse<'a> for AvcTimingAndHrdDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AvcTimingAndHrdDescriptor",
            "unexpected tag for AVC_timing_and_HRD_descriptor",
        )?;
        if body.len() < (FIXED_BODY_LEN as usize) {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "AVC_timing_and_HRD_descriptor too short",
            });
        }

        let b0 = body[0];
        let hrd_management_valid_flag = (b0 & 0x80) != 0;
        let picture_and_timing_info_present = (b0 & 0x01) != 0;

        let (picture_timing, flags_offset) = if picture_and_timing_info_present {
            // picture_and_timing block: 1 byte (90kHz_flag + reserved) + optional 8 bytes (N+K)
            if body.len() < 2 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "AVC_timing_and_HRD_descriptor too short for picture_timing block",
                });
            }
            let b1 = body[1];
            let _90khz_flag = (b1 & 0x80) != 0;

            let (n, k, nk_len) = if _90khz_flag {
                (None, None, 0)
            } else {
                if body.len() < 10 {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "AVC_timing_and_HRD_descriptor too short for N/K fields",
                    });
                }
                let n = u32::from_be_bytes([body[2], body[3], body[4], body[5]]);
                let k = u32::from_be_bytes([body[6], body[7], body[8], body[9]]);
                (Some(n), Some(k), 8)
            };

            let num_units_offset = 2 + nk_len;
            if body.len() < num_units_offset + 4 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "AVC_timing_and_HRD_descriptor too short for num_units_in_tick",
                });
            }
            let num_units_in_tick = u32::from_be_bytes([
                body[num_units_offset],
                body[num_units_offset + 1],
                body[num_units_offset + 2],
                body[num_units_offset + 3],
            ]);

            let timing = AvcPictureTiming {
                _90khz_flag,
                n,
                k,
                num_units_in_tick,
                reserved_7: b1 & RESERVED_7_MASK,
            };
            (Some(timing), num_units_offset + 4)
        } else {
            (None, 1)
        };

        if body.len() != flags_offset + 1 {
            // The descriptor ends at the flags byte (Table 2-117); trailing
            // bytes have no wire meaning and used to be dropped on
            // re-serialize (r03-W9).
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "AVC_timing_and_HRD_descriptor body length disagrees with its flags",
            });
        }
        let flags_byte = body[flags_offset];
        let fixed_frame_rate_flag = (flags_byte & 0x80) != 0;
        let temporal_poc_flag = (flags_byte & 0x40) != 0;
        let picture_to_display_conversion_flag = (flags_byte & 0x20) != 0;

        Ok(Self {
            hrd_management_valid_flag,
            picture_timing,
            fixed_frame_rate_flag,
            temporal_poc_flag,
            picture_to_display_conversion_flag,
            reserved_6: (b0 >> 1) & RESERVED_6_MASK,
            reserved_5: flags_byte & RESERVED_5_MASK,
        })
    }
}

impl Serialize for AvcTimingAndHrdDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let body_len = 1u8
            + if let Some(ref pt) = self.picture_timing {
                if pt._90khz_flag {
                    1u8 + 4 // 90kHz_flag byte + num_units_in_tick
                } else {
                    1u8 + 8 + 4 // 90kHz_flag byte + N(4) + K(4) + num_units_in_tick
                }
            } else {
                0
            };
        HEADER_LEN + body_len as usize + 1 // +1 for the trailing flags byte
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let body_len = (len - HEADER_LEN) as u8;
        buf[0] = TAG;
        buf[1] = body_len;

        let picture_and_timing_info_present = self.picture_timing.is_some();
        buf[HEADER_LEN] = ((self.hrd_management_valid_flag as u8) << 7)
            | ((self.reserved_6 & RESERVED_6_MASK) << 1)
            | (if picture_and_timing_info_present {
                1
            } else {
                0
            });

        let mut pos = HEADER_LEN + 1;
        if let Some(ref pt) = self.picture_timing {
            buf[pos] = if pt._90khz_flag { 0x80 } else { 0x00 } | (pt.reserved_7 & RESERVED_7_MASK);
            pos += 1;
            if !pt._90khz_flag {
                // N and K are both on the wire whenever the 90 kHz flag is
                // clear; fabricating 0 would misframe (r03-W5).
                let n = pt.n.ok_or(Error::ValueOutOfRange {
                    field: "N",
                    reason: "required when _90khz_flag is false",
                })?;
                let k = pt.k.ok_or(Error::ValueOutOfRange {
                    field: "K",
                    reason: "required when _90khz_flag is false",
                })?;
                buf[pos..pos + 4].copy_from_slice(&n.to_be_bytes());
                buf[pos + 4..pos + 8].copy_from_slice(&k.to_be_bytes());
                pos += 8;
            }
            buf[pos..pos + 4].copy_from_slice(&pt.num_units_in_tick.to_be_bytes());
            pos += 4;
        }

        buf[pos] = ((self.fixed_frame_rate_flag as u8) << 7)
            | ((self.temporal_poc_flag as u8) << 6)
            | ((self.picture_to_display_conversion_flag as u8) << 5)
            | (self.reserved_5 & RESERVED_5_MASK);
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for AvcTimingAndHrdDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AVC_TIMING_AND_HRD";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_no_timing() {
        let orig = AvcTimingAndHrdDescriptor::new(true, None, false, true, false);
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = AvcTimingAndHrdDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn round_trip_90khz() {
        let orig = AvcTimingAndHrdDescriptor::new(
            false,
            Some(AvcPictureTiming::new(true, None, None, 0xDEADBEEF)),
            true,
            false,
            true,
        );
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = AvcTimingAndHrdDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn round_trip_nk() {
        let orig = AvcTimingAndHrdDescriptor::new(
            true,
            Some(AvcPictureTiming::new(
                false,
                Some(0x12345678),
                Some(0x9ABCDEF0),
                0x0FEDCBA9,
            )),
            false,
            false,
            false,
        );
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = AvcTimingAndHrdDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = AvcTimingAndHrdDescriptor::parse(&[0x02, 1, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        let err = AvcTimingAndHrdDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }
    #[test]
    fn serialize_rejects_missing_n_or_k_when_not_90khz() {
        // r03-W5: N/K used to be `unwrap_or(0)`-ed onto the wire when the
        // 90 kHz flag was clear, silently serializing fabricated zeros.
        let d = AvcTimingAndHrdDescriptor::new(
            true,
            Some(AvcPictureTiming::new(false, None, Some(1), 2)),
            false,
            false,
            false,
        );
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::ValueOutOfRange { field: "N", .. }
        ));
    }

    #[test]
    fn parse_rejects_trailing_bytes_after_flags_byte() {
        // The descriptor ends at the flags byte (Table 2-117); a byte after
        // it has no wire meaning and used to be dropped on re-serialize
        // (r03-W9).
        // hrd_management_valid_flag=0, no picture timing, flags byte, extra.
        let bytes = [TAG, 3, 0x00, 0x00, 0xFF];
        assert!(matches!(
            AvcTimingAndHrdDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    /// H.222.0 Table 2-93. All three reserved regions are preserved (zero and
    /// representative values); the old normalize-to-ones behaviour is gone.
    #[test]
    fn avc_timing_and_hrd_reserved_preserved() {
        for r6 in [0u8, 1, 0x15, 0x2A, 0x3F] {
            for r7 in [0u8, 1, 0x2A, 0x55, 0x7F] {
                for r5 in [0u8, 1, 0x0A, 0x15, 0x1F] {
                    let bytes = [
                        TAG,
                        15,
                        0x80 | (r6 << 1) | 1,
                        r7,
                        0,
                        0,
                        0,
                        1,
                        0,
                        0,
                        0,
                        2,
                        0,
                        0,
                        0,
                        3,
                        0xA0 | r5,
                    ];
                    let d = AvcTimingAndHrdDescriptor::parse(&bytes).unwrap();
                    assert_eq!((d.reserved_6, d.reserved_5), (r6, r5));
                    assert_eq!(d.picture_timing.as_ref().unwrap().reserved_7, r7);
                    let mut out = vec![0u8; d.serialized_len()];
                    d.serialize_into(&mut out).unwrap();
                    assert_eq!(out, bytes);
                }
            }
        }
    }

    #[test]
    fn avc_timing_and_hrd_default_is_spec_value() {
        let d = AvcTimingAndHrdDescriptor::new(
            true,
            Some(AvcPictureTiming::new(false, Some(1), Some(2), 3)),
            true,
            false,
            true,
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out,
            [
                TAG, 15, 0xFF, 0x7F, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0xBF
            ]
        );
    }
}
