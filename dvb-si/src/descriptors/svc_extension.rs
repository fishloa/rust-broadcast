//! SVC Extension Descriptor — ISO/IEC 13818-1 §2.6.76, Table 2-99 (tag 0x30).
//!
//! Carries SVC (Scalable Video Coding, H.264 Annex G) extension parameters
//! for a video elementary stream.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::len::need_out;
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for SVC_extension_descriptor.
pub const TAG: u8 = 0x30;
const HEADER_LEN: usize = 2;
/// Default of the 5-bit `reserved` after `dependency_id` (H.222.0 Table 2-99):
/// clause 2.1 "reserved" = `1` (all editions 06/2012-08/2023, audit §5;
/// confirmed by the TSDuck oracle PMT).
pub const RESERVED_5_DEFAULT: ReservedOnes<RESERVED_5_BITS> = ReservedOnes::<RESERVED_5_BITS>::FILL;
/// Width of the `reserved` field after `dependency_id` (5 bits).
const RESERVED_5_BITS: u32 = 5;
/// Default of the trailing 1-bit `reserved` after `no_sei_nal_unit_present`
/// (same rule).
pub const RESERVED_1_DEFAULT: ReservedOnes<RESERVED_1_BITS> = ReservedOnes::<RESERVED_1_BITS>::FILL;
/// Width of the trailing `reserved` field (1 bit).
const RESERVED_1_BITS: u32 = 1;
const RESERVED_5_MASK: u8 = 0x1F;
const RESERVED_1_MASK: u8 = 0x01;
const BODY_LEN: u8 = 13;
/// Body offset of `height`.
const HEIGHT_OFFSET: usize = 2;
/// Body offset of `frame_rate`.
const FRAME_RATE_OFFSET: usize = 4;
/// Body offset of `average_bitrate`.
const AVERAGE_BITRATE_OFFSET: usize = 6;
/// Body offset of `maximum_bitrate`.
const MAXIMUM_BITRATE_OFFSET: usize = 8;
/// Body offset of the `dependency_id`/reserved byte.
const DEPENDENCY_FLAGS_OFFSET: usize = 10;
/// Body offset of the quality-id byte.
const QUALITY_FLAGS_OFFSET: usize = 11;
/// Body offset of the temporal-id byte.
const TEMPORAL_FLAGS_OFFSET: usize = 12;
/// Shift of `dependency_id` in its byte.
const DEPENDENCY_ID_SHIFT: u8 = 5;
/// Mask of the 3-bit `dependency_id`.
const DEPENDENCY_ID_MASK: u8 = 0x07;
/// Shift of `quality_id_start` in its byte.
const QUALITY_ID_START_SHIFT: u8 = 4;
/// Mask of a 4-bit quality id.
const QUALITY_ID_MASK: u8 = 0x0F;
/// Shift of `temporal_id_start` in its byte.
const TEMPORAL_ID_START_SHIFT: u8 = 5;
/// Shift of `temporal_id_end` in its byte.
const TEMPORAL_ID_END_SHIFT: u8 = 2;
/// Mask of a 3-bit temporal id.
const TEMPORAL_ID_MASK: u8 = 0x07;
/// `no_sei_nal_unit_present` bit of the temporal-id byte.
const NO_SEI_MASK: u8 = 0x02;
/// Shift of `no_sei_nal_unit_present` in the temporal-id byte.
const NO_SEI_SHIFT: u8 = 1;

/// SVC Extension Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SvcExtensionDescriptor {
    /// Width of the SVC stream in pixels.
    pub width: u16,
    /// Height of the SVC stream in pixels.
    pub height: u16,
    /// Frame rate in frames/256 seconds.
    pub frame_rate: u16,
    /// Average bitrate in kbit/s (`0` means unspecified).
    pub average_bitrate: u16,
    /// Maximum bitrate in kbit/s (`0` means unspecified).
    pub maximum_bitrate: u16,
    /// Dependency ID (SVC spatial/quality dependency layer).
    pub dependency_id: u8,
    /// Quality ID start (inclusive).
    pub quality_id_start: u8,
    /// Quality ID end (inclusive).
    pub quality_id_end: u8,
    /// Temporal ID start (inclusive).
    pub temporal_id_start: u8,
    /// Temporal ID end (inclusive).
    pub temporal_id_end: u8,
    /// No SEI NAL unit present flag.
    pub no_sei_nal_unit_present: bool,
    /// `reserved(5)` after `dependency_id` (5 bits). Captured on parse,
    /// re-emitted verbatim; defaults to [`RESERVED_5_DEFAULT`].
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_5: ReservedOnes<RESERVED_5_BITS>,
    /// Trailing `reserved(1)` (1 bit). Captured on parse, re-emitted verbatim;
    /// defaults to [`RESERVED_1_DEFAULT`].
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_1: ReservedOnes<RESERVED_1_BITS>,
}

impl SvcExtensionDescriptor {
    /// Build with the picture geometry; bitrates unspecified (`0`), layer ids
    /// 0, SEI flag clear and the spec-default reserved bits. Set the remaining
    /// fields directly.
    #[must_use]
    pub const fn new(width: u16, height: u16, frame_rate: u16) -> Self {
        Self {
            width,
            height,
            frame_rate,
            average_bitrate: 0,
            maximum_bitrate: 0,
            dependency_id: 0,
            quality_id_start: 0,
            quality_id_end: 0,
            temporal_id_start: 0,
            temporal_id_end: 0,
            no_sei_nal_unit_present: false,
            reserved_5: RESERVED_5_DEFAULT,
            reserved_1: RESERVED_1_DEFAULT,
        }
    }
}

impl Default for SvcExtensionDescriptor {
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

impl<'a> Parse<'a> for SvcExtensionDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SvcExtensionDescriptor",
            "unexpected tag for SVC_extension_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "SVC_extension_descriptor length must equal 13",
            });
        }
        let width = u16::from_be_bytes([body[0], body[1]]);
        let height = u16::from_be_bytes([body[HEIGHT_OFFSET], body[HEIGHT_OFFSET + 1]]);
        let frame_rate = u16::from_be_bytes([body[FRAME_RATE_OFFSET], body[FRAME_RATE_OFFSET + 1]]);
        let average_bitrate = u16::from_be_bytes([
            body[AVERAGE_BITRATE_OFFSET],
            body[AVERAGE_BITRATE_OFFSET + 1],
        ]);
        let maximum_bitrate = u16::from_be_bytes([
            body[MAXIMUM_BITRATE_OFFSET],
            body[MAXIMUM_BITRATE_OFFSET + 1],
        ]);
        let b10 = body[DEPENDENCY_FLAGS_OFFSET];
        let dependency_id = (b10 >> DEPENDENCY_ID_SHIFT) & DEPENDENCY_ID_MASK;
        // reserved: top 5 bits are reserved — we accept any value for round-trip
        let b11 = body[QUALITY_FLAGS_OFFSET];
        let quality_id_start = (b11 >> QUALITY_ID_START_SHIFT) & QUALITY_ID_MASK;
        let quality_id_end = b11 & QUALITY_ID_MASK;
        let b12 = body[TEMPORAL_FLAGS_OFFSET];
        let temporal_id_start = (b12 >> TEMPORAL_ID_START_SHIFT) & TEMPORAL_ID_MASK;
        let temporal_id_end = (b12 >> TEMPORAL_ID_END_SHIFT) & TEMPORAL_ID_MASK;
        let no_sei_nal_unit_present = (b12 & NO_SEI_MASK) != 0;
        // reserved: bit 0 is reserved
        Ok(Self {
            width,
            height,
            frame_rate,
            average_bitrate,
            maximum_bitrate,
            dependency_id,
            quality_id_start,
            quality_id_end,
            temporal_id_start,
            temporal_id_end,
            no_sei_nal_unit_present,
            reserved_5: ReservedOnes::from_bits(u64::from(b10 & RESERVED_5_MASK)),
            reserved_1: ReservedOnes::from_bits(u64::from(b12 & RESERVED_1_MASK)),
        })
    }
}

impl Serialize for SvcExtensionDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN as usize
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN..HEADER_LEN + size_of::<u16>()].copy_from_slice(&self.width.to_be_bytes());
        buf[HEADER_LEN + HEIGHT_OFFSET..HEADER_LEN + HEIGHT_OFFSET + size_of::<u16>()]
            .copy_from_slice(&self.height.to_be_bytes());
        buf[HEADER_LEN + FRAME_RATE_OFFSET..HEADER_LEN + FRAME_RATE_OFFSET + size_of::<u16>()]
            .copy_from_slice(&self.frame_rate.to_be_bytes());
        buf[HEADER_LEN + AVERAGE_BITRATE_OFFSET..HEADER_LEN + AVERAGE_BITRATE_OFFSET + size_of::<u16>()]
            .copy_from_slice(&self.average_bitrate.to_be_bytes());
        buf[HEADER_LEN + MAXIMUM_BITRATE_OFFSET..HEADER_LEN + MAXIMUM_BITRATE_OFFSET + size_of::<u16>()]
            .copy_from_slice(&self.maximum_bitrate.to_be_bytes());
        buf[HEADER_LEN + DEPENDENCY_FLAGS_OFFSET] = (broadcast_common::len::fit_mask(
            self.dependency_id,
            DEPENDENCY_ID_MASK,
            "dependency_id",
        )? << DEPENDENCY_ID_SHIFT)
            | (self.reserved_5.get() as u8);
        buf[HEADER_LEN + QUALITY_FLAGS_OFFSET] = (broadcast_common::len::fit_mask(
            self.quality_id_start,
            QUALITY_ID_MASK,
            "quality_id_start",
        )? << QUALITY_ID_START_SHIFT)
            | broadcast_common::len::fit_mask(self.quality_id_end, QUALITY_ID_MASK, "quality_id_end")?;
        buf[HEADER_LEN + TEMPORAL_FLAGS_OFFSET] = (broadcast_common::len::fit_mask(
            self.temporal_id_start,
            TEMPORAL_ID_MASK,
            "temporal_id_start",
        )? << TEMPORAL_ID_START_SHIFT)
            | (broadcast_common::len::fit_mask(
                self.temporal_id_end,
                TEMPORAL_ID_MASK,
                "temporal_id_end",
            )? << TEMPORAL_ID_END_SHIFT)
            | ((self.no_sei_nal_unit_present as u8) << NO_SEI_SHIFT)
            | (self.reserved_1.get() as u8);
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for SvcExtensionDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SVC_EXTENSION";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse_extracts_fields() {
        let bytes = [
            TAG, 13, 0x07, 0x80, // width = 1920
            0x04, 0x38, // height = 1080
            0x00, 0x3C, // frame_rate = 60
            0x00, 0x64, // average_bitrate = 100
            0x00, 0xC8, // maximum_bitrate = 200
            0x20, // dependency_id=1, reserved=0
            0x01, // quality_id_start=0, quality_id_end=1
            0x06, // temporal_id_start=0, temporal_id_end=1, no_sei=1, reserved=0
        ];
        let d = SvcExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.width, 1920);
        assert_eq!(d.height, 1080);
        assert_eq!(d.frame_rate, 60);
        assert_eq!(d.average_bitrate, 100);
        assert_eq!(d.maximum_bitrate, 200);
        assert_eq!(d.dependency_id, 1);
        assert_eq!(d.quality_id_start, 0);
        assert_eq!(d.quality_id_end, 1);
        assert_eq!(d.temporal_id_start, 0);
        assert_eq!(d.temporal_id_end, 1);
        assert!(d.no_sei_nal_unit_present);
    }

    #[test]
    fn parse_max_bit_fields() {
        let bytes = [
            TAG, 13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xE0, // dependency_id=7
            0xFF, // quality_id_start=15, quality_id_end=15
            0xFE, // temporal_id_start=7, temporal_id_end=7, no_sei=1
        ];
        let d = SvcExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.dependency_id, 7);
        assert_eq!(d.quality_id_start, 15);
        assert_eq!(d.quality_id_end, 15);
        assert_eq!(d.temporal_id_start, 7);
        assert_eq!(d.temporal_id_end, 7);
        assert!(d.no_sei_nal_unit_present);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = SvcExtensionDescriptor::parse(&[0x02, 13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            .unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = SvcExtensionDescriptor::parse(&[TAG, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            .unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let mut d = SvcExtensionDescriptor::new(1280, 720, 50);
        d.average_bitrate = 5000;
        d.maximum_bitrate = 10000;
        d.dependency_id = 2;
        d.quality_id_start = 3;
        d.quality_id_end = 5;
        d.temporal_id_start = 1;
        d.temporal_id_end = 3;
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = SvcExtensionDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = SvcExtensionDescriptor::default();
        let mut tiny = vec![0u8; 5];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall(..)));
    }

    /// H.222.0 Table 2-99. Both reserved regions are preserved verbatim (the
    /// earlier test that expected them to be wiped/normalized is replaced).
    #[test]
    fn svc_extension_reserved_preserved() {
        for r5 in [0u8, 1, 0x0A, 0x15, 0x1F] {
            for r1 in 0u8..=1 {
                let bytes = [
                    TAG,
                    13,
                    0x07,
                    0x80,
                    0x04,
                    0x38,
                    0x00,
                    0x3C,
                    0x00,
                    0x64,
                    0x00,
                    0xC8,
                    0x20 | r5,
                    0x01,
                    0x06 | r1,
                ];
                let d = SvcExtensionDescriptor::parse(&bytes).unwrap();
                assert_eq!(
                    (d.reserved_5, d.reserved_1),
                    (
                        ReservedOnes::from_bits(u64::from(r5)),
                        ReservedOnes::from_bits(u64::from(r1))
                    )
                );
                let mut out = vec![0u8; d.serialized_len()];
                d.serialize_into(&mut out).unwrap();
                assert_eq!(out, bytes);
            }
        }
    }

    #[test]
    fn svc_extension_default_is_spec_value() {
        let mut d = SvcExtensionDescriptor::new(0x0780, 0x0438, 0x003C);
        d.average_bitrate = 0x64;
        d.maximum_bitrate = 0xC8;
        d.dependency_id = 1;
        d.quality_id_end = 1;
        d.temporal_id_end = 1;
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out,
            [
                TAG, 13, 0x07, 0x80, 0x04, 0x38, 0x00, 0x3C, 0x00, 0x64, 0x00, 0xC8, 0x3F, 0x01,
                0x05
            ]
        );
    }

    /// Each reserved field is width-limited by its type, so a wider value is
    /// unrepresentable and cannot spill into the adjacent syntax bits.
    #[test]
    fn reserved_fields_cannot_hold_more_than_their_width() {
        let d = SvcExtensionDescriptor {
            reserved_5: ReservedOnes::from_bits(0xFF),
            reserved_1: ReservedOnes::from_bits(0xFF),
            ..SvcExtensionDescriptor::new(0, 0, 0)
        };
        assert_eq!(d.reserved_5.get(), 0x1F);
        assert_eq!(d.reserved_1.get(), 0x01);
    }
}
