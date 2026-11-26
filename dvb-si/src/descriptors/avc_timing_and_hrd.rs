//! AVC Timing and HRD Descriptor — ISO/IEC 13818-1 §2.6.66 (tag 0x2A).
//!
//! Provides encoding-parameters timestamps, HRD management validity,
//! and frame-rate conversion flags for an AVC/H.264 elementary stream.
//! The picture_and_timing_info block is conditional on
//! `picture_and_timing_info_present`; the inner N/K fields within it are
//! conditional on `90kHz_flag == 0`.

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::wire::U32_LEN;
use broadcast_core::len::need_out;
use broadcast_core::reserved::ReservedOnes;
use broadcast_core::{Parse, Serialize};

/// Descriptor tag for AVC_timing_and_HRD_descriptor.
pub const TAG: u8 = 0x2A;
const HEADER_LEN: usize = 2;
/// Default of the 6-bit `reserved` in byte 0 (H.222.0 Table 2-93): clause 2.1
/// "reserved" = `1` (all editions 06/2012-08/2023, audit §5; confirmed by the
/// TSDuck oracle PMT).
pub const RESERVED_6_DEFAULT: ReservedOnes<RESERVED_FIRST_BYTE_BITS> =
    ReservedOnes::<RESERVED_FIRST_BYTE_BITS>::FILL;
/// Default of the 7-bit `reserved` after `90kHz_flag` (same rule).
pub const RESERVED_7_DEFAULT: ReservedOnes<RESERVED_AFTER_90KHZ_FLAG_BITS> =
    ReservedOnes::<RESERVED_AFTER_90KHZ_FLAG_BITS>::FILL;
/// Default of the trailing 5-bit `reserved` after the frame-rate flags (same
/// rule).
pub const RESERVED_5_DEFAULT: ReservedOnes<RESERVED_TRAILING_BITS> =
    ReservedOnes::<RESERVED_TRAILING_BITS>::FILL;
/// Width of the `reserved` in the first body byte (6 bits).
pub const RESERVED_FIRST_BYTE_BITS: u32 = 6;
/// Width of the `reserved` after `90kHz_flag` (7 bits).
pub const RESERVED_AFTER_90KHZ_FLAG_BITS: u32 = 7;
/// Width of the trailing `reserved` after the frame-rate flags (5 bits).
pub const RESERVED_TRAILING_BITS: u32 = 5;
const FIXED_BODY_LEN: u8 = 1; // first byte always present (hrd_management_valid_flag..picture_and_timing_info_present)
/// Width of a 32-bit wire field, bytes.
/// Width of the `N` + `K` pair, bytes.
const N_AND_K_LEN: usize = U32_LEN + U32_LEN;
/// Width of the `90kHz_flag`/reserved byte, bytes.
const PICTURE_TIMING_PREFIX_LEN: usize = 2;
/// Shortest picture-timing block that carries `N`/`K`.
const PICTURE_TIMING_NK_LEN: usize = PICTURE_TIMING_PREFIX_LEN + N_AND_K_LEN;
/// Offset of `N` in the descriptor body.
const N_OFFSET: usize = PICTURE_TIMING_PREFIX_LEN;
/// Offset of `K` in the descriptor body.
const K_OFFSET: usize = N_OFFSET + U32_LEN;
/// Offset of `num_units_in_tick` after an explicit `N`/`K` pair.
const NUM_UNITS_IN_TICK_OFFSET: usize = K_OFFSET + U32_LEN;
const HRD_MANAGEMENT_VALID_FLAG_MASK: u8 = 0x80;
const HRD_MANAGEMENT_VALID_FLAG_SHIFT: u8 = 7;
const PICTURE_AND_TIMING_INFO_PRESENT_MASK: u8 = 0x01;
const RESERVED_6_SHIFT: u8 = 1;
const CLOCK_90KHZ_FLAG_MASK: u8 = 0x80;
const FIXED_FRAME_RATE_FLAG_MASK: u8 = 0x80;
const FIXED_FRAME_RATE_FLAG_SHIFT: u8 = 7;
const TEMPORAL_POC_FLAG_MASK: u8 = 0x40;
const TEMPORAL_POC_FLAG_SHIFT: u8 = 6;
const PICTURE_TO_DISPLAY_CONVERSION_FLAG_MASK: u8 = 0x20;
const PICTURE_TO_DISPLAY_CONVERSION_FLAG_SHIFT: u8 = 5;

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
    pub reserved_7: ReservedOnes<RESERVED_AFTER_90KHZ_FLAG_BITS>,
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
    pub reserved_6: ReservedOnes<RESERVED_FIRST_BYTE_BITS>,
    /// Trailing `reserved(5)` after the flags (5 bits, right-aligned).
    /// Captured on parse, re-emitted verbatim; defaults to
    /// [`RESERVED_5_DEFAULT`].
    pub reserved_5: ReservedOnes<RESERVED_TRAILING_BITS>,
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
        let hrd_management_valid_flag = (b0 & HRD_MANAGEMENT_VALID_FLAG_MASK) != 0;
        let picture_and_timing_info_present = (b0 & PICTURE_AND_TIMING_INFO_PRESENT_MASK) != 0;

        let (picture_timing, flags_offset) = if picture_and_timing_info_present {
            // picture_and_timing block: 1 byte (90kHz_flag + reserved) + optional 8 bytes (N+K)
            if body.len() < PICTURE_TIMING_PREFIX_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "AVC_timing_and_HRD_descriptor too short for picture_timing block",
                });
            }
            let b1 = body[FIXED_BODY_LEN as usize];
            let _90khz_flag = (b1 & CLOCK_90KHZ_FLAG_MASK) != 0;

            let (n, k, nk_len) = if _90khz_flag {
                (None, None, 0)
            } else {
                if body.len() < PICTURE_TIMING_NK_LEN {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "AVC_timing_and_HRD_descriptor too short for N/K fields",
                    });
                }
                let mut n_bytes = [0u8; U32_LEN];
                n_bytes.copy_from_slice(&body[N_OFFSET..K_OFFSET]);
                let mut k_bytes = [0u8; U32_LEN];
                k_bytes.copy_from_slice(&body[K_OFFSET..NUM_UNITS_IN_TICK_OFFSET]);
                let n = u32::from_be_bytes(n_bytes);
                let k = u32::from_be_bytes(k_bytes);
                (Some(n), Some(k), N_AND_K_LEN)
            };

            let num_units_offset = PICTURE_TIMING_PREFIX_LEN + nk_len;
            if body.len() < num_units_offset + U32_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "AVC_timing_and_HRD_descriptor too short for num_units_in_tick",
                });
            }
            let mut num_units_bytes = [0u8; U32_LEN];
            num_units_bytes.copy_from_slice(&body[num_units_offset..num_units_offset + U32_LEN]);
            let num_units_in_tick = u32::from_be_bytes(num_units_bytes);

            let timing = AvcPictureTiming {
                _90khz_flag,
                n,
                k,
                num_units_in_tick,
                reserved_7: ReservedOnes::from_bits(u64::from(b1)),
            };
            (Some(timing), num_units_offset + U32_LEN)
        } else {
            (None, FIXED_BODY_LEN as usize)
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
        let fixed_frame_rate_flag = (flags_byte & FIXED_FRAME_RATE_FLAG_MASK) != 0;
        let temporal_poc_flag = (flags_byte & TEMPORAL_POC_FLAG_MASK) != 0;
        let picture_to_display_conversion_flag =
            (flags_byte & PICTURE_TO_DISPLAY_CONVERSION_FLAG_MASK) != 0;

        Ok(Self {
            hrd_management_valid_flag,
            picture_timing,
            fixed_frame_rate_flag,
            temporal_poc_flag,
            picture_to_display_conversion_flag,
            reserved_6: ReservedOnes::from_bits(u64::from(b0 >> RESERVED_6_SHIFT)),
            reserved_5: ReservedOnes::from_bits(u64::from(flags_byte)),
        })
    }
}

impl Serialize for AvcTimingAndHrdDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let body_len = 1u8
            + if let Some(ref pt) = self.picture_timing {
                if pt._90khz_flag {
                    FIXED_BODY_LEN + U32_LEN as u8 // 90kHz_flag byte + num_units_in_tick
                } else {
                    FIXED_BODY_LEN + N_AND_K_LEN as u8 + U32_LEN as u8 // 90kHz_flag byte + N(4) + K(4) + num_units_in_tick
                }
            } else {
                0
            };
        HEADER_LEN + body_len as usize + 1 // +1 for the trailing flags byte
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let body_len = (len - HEADER_LEN) as u8;
        buf[0] = TAG;
        buf[1] = body_len;

        let picture_and_timing_info_present = self.picture_timing.is_some();
        buf[HEADER_LEN] = (u8::from(self.hrd_management_valid_flag)
            << HRD_MANAGEMENT_VALID_FLAG_SHIFT)
            | ((self.reserved_6.get() as u8) << RESERVED_6_SHIFT)
            | (if picture_and_timing_info_present {
                1
            } else {
                0
            });

        let mut pos = HEADER_LEN + 1;
        if let Some(ref pt) = self.picture_timing {
            buf[pos] = if pt._90khz_flag {
                CLOCK_90KHZ_FLAG_MASK
            } else {
                0
            } | pt.reserved_7.get() as u8;
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
                buf[pos..pos + U32_LEN].copy_from_slice(&n.to_be_bytes());
                buf[pos + U32_LEN..pos + N_AND_K_LEN].copy_from_slice(&k.to_be_bytes());
                pos += N_AND_K_LEN;
            }
            buf[pos..pos + U32_LEN].copy_from_slice(&pt.num_units_in_tick.to_be_bytes());
            pos += U32_LEN;
        }

        buf[pos] = (u8::from(self.fixed_frame_rate_flag) << FIXED_FRAME_RATE_FLAG_SHIFT)
            | (u8::from(self.temporal_poc_flag) << TEMPORAL_POC_FLAG_SHIFT)
            | (u8::from(self.picture_to_display_conversion_flag)
                << PICTURE_TO_DISPLAY_CONVERSION_FLAG_SHIFT)
            | self.reserved_5.get() as u8;
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
    use alloc::vec;

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
                    assert_eq!(
                        (d.reserved_6.get(), d.reserved_5.get()),
                        (u64::from(r6), u64::from(r5))
                    );
                    assert_eq!(
                        d.picture_timing.as_ref().unwrap().reserved_7.get(),
                        u64::from(r7)
                    );
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
