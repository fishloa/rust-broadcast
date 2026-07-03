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
const FIXED_BODY_LEN: u8 = 1; // first byte always present (hrd_management_valid_flag..picture_and_timing_info_present)

/// Picture and timing info block — present when `picture_and_timing_info_present` is true.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AvcPictureTiming {
    /// 1 — 90 kHz clock; 0 — N/K clock used.
    pub _90khz_flag: bool,
    /// N parameter (when `_90khz_flag` is false).
    pub n: Option<u32>,
    /// K parameter (when `_90khz_flag` is false).
    pub k: Option<u32>,
    /// num_units_in_tick.
    pub num_units_in_tick: u32,
}

/// AVC Timing and HRD Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
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
            };
            (Some(timing), num_units_offset + 4)
        } else {
            (None, 1)
        };

        if body.len() < flags_offset + 1 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "AVC_timing_and_HRD_descriptor too short for trailing flags",
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
            | (if picture_and_timing_info_present {
                1
            } else {
                0
            });

        let mut pos = HEADER_LEN + 1;
        if let Some(ref pt) = self.picture_timing {
            buf[pos] = if pt._90khz_flag { 0x80 } else { 0x00 };
            pos += 1;
            if !pt._90khz_flag {
                let n = pt.n.unwrap_or(0);
                let k = pt.k.unwrap_or(0);
                buf[pos..pos + 4].copy_from_slice(&n.to_be_bytes());
                buf[pos + 4..pos + 8].copy_from_slice(&k.to_be_bytes());
                pos += 8;
            }
            buf[pos..pos + 4].copy_from_slice(&pt.num_units_in_tick.to_be_bytes());
            pos += 4;
        }

        buf[pos] = ((self.fixed_frame_rate_flag as u8) << 7)
            | ((self.temporal_poc_flag as u8) << 6)
            | ((self.picture_to_display_conversion_flag as u8) << 5);
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
        let orig = AvcTimingAndHrdDescriptor {
            hrd_management_valid_flag: true,
            picture_timing: None,
            fixed_frame_rate_flag: false,
            temporal_poc_flag: true,
            picture_to_display_conversion_flag: false,
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = AvcTimingAndHrdDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn round_trip_90khz() {
        let orig = AvcTimingAndHrdDescriptor {
            hrd_management_valid_flag: false,
            picture_timing: Some(AvcPictureTiming {
                _90khz_flag: true,
                n: None,
                k: None,
                num_units_in_tick: 0xDEADBEEF,
            }),
            fixed_frame_rate_flag: true,
            temporal_poc_flag: false,
            picture_to_display_conversion_flag: true,
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = AvcTimingAndHrdDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn round_trip_nk() {
        let orig = AvcTimingAndHrdDescriptor {
            hrd_management_valid_flag: true,
            picture_timing: Some(AvcPictureTiming {
                _90khz_flag: false,
                n: Some(0x12345678),
                k: Some(0x9ABCDEF0),
                num_units_in_tick: 0x0FEDCBA9,
            }),
            fixed_frame_rate_flag: false,
            temporal_poc_flag: false,
            picture_to_display_conversion_flag: false,
        };
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
}
