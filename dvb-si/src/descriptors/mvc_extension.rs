//! MVC Extension Descriptor — ISO/IEC 13818-1 §2.6.78, Table 2-100 (tag 0x31).
//!
//! Carries MVC (Multiview Video Coding, H.264 Annex H) extension parameters
//! for a video elementary stream.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::len::need_out;
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for MVC_extension_descriptor.
pub const TAG: u8 = 0x31;
const HEADER_LEN: usize = 2;
/// Default of the 2-bit `reserved` before `view_order_index_min` (H.222.0
/// Table 2-100), bits `[5:4]` of byte 4: clause 2.1 "reserved" = `1` (editions
/// 10/2014-08/2023, audit §4 row 9 / §5; the 2014 edition carved the two view
/// flags out of the older `reserved(4)`, typed here; confirmed by the TSDuck
/// oracle PMT).
pub const RESERVED_DEFAULT: ReservedOnes<RESERVED_BITS> = ReservedOnes::<RESERVED_BITS>::FILL;
/// Width of the `reserved` field ahead of `view_order_index_min` (2 bits).
pub const RESERVED_BITS: u32 = 2;
/// Mask and shift of `reserved(2)` within byte 4.
const RESERVED_MASK: u8 = 0x03;
/// Shift of the 2-bit `reserved` run within its byte.
const RESERVED_SHIFT: u8 = 4;
const BODY_LEN: u8 = 8;
/// Body offset of `maximum_bitrate`.
const MAXIMUM_BITRATE_OFFSET: usize = 2;
/// Body offset of the view-flags byte.
const VIEW_FLAGS_OFFSET: usize = 4;
/// Body offset of the `view_order_index` continuation byte.
const VIEW_ORDER_CONTINUATION_OFFSET: usize = 5;
/// Body offset of the high `view_order_index_max` byte.
const VIEW_ORDER_MAX_HI_OFFSET: usize = 6;
/// Body offset of the temporal/flags byte.
const TEMPORAL_FLAGS_OFFSET: usize = 7;
/// `view_association_not_present` bit of the view-flags byte.
const VIEW_ASSOCIATION_MASK: u8 = 0x80;
/// `base_view_is_left_eyeview` bit of the view-flags byte.
const BASE_VIEW_LEFT_EYE_MASK: u8 = 0x40;
/// Mask of the high `view_order_index_min` bits in the view-flags byte.
const VIEW_ORDER_MIN_HI_MASK: u8 = 0x0F;
/// Mask of the high `view_order_index_max` bits in the continuation byte.
const VIEW_ORDER_MAX_HI_MASK: u8 = 0x03;
/// Mask of the low `view_order_index_min` bits in the continuation byte.
const VIEW_ORDER_MIN_LO_MASK: u8 = 0x3F;
/// Wire width of `view_order_index_min` / `view_order_index_max`.
const VIEW_ORDER_INDEX_BITS: u32 = 10;
/// Shift of the high `view_order_index_min` bits.
const VIEW_ORDER_MIN_HI_SHIFT: u16 = 6;
/// Shift of the low `view_order_index_min` bits.
const VIEW_ORDER_MIN_LO_SHIFT: u16 = 2;
/// Shift of the high `view_order_index_max` bits.
const VIEW_ORDER_MAX_HI_SHIFT: u16 = 8;
/// Shift of the high bytes of both view order indices in the serializer.
const VIEW_ORDER_HI_SHIFT: u16 = 6;
/// Mask of the low `view_order_index_min` bits in the serializer.
const VIEW_ORDER_MIN_LO_SERIALIZE_MASK: u16 = 0x3F;
/// Shift of the packed view order index bytes in the serializer.
const VIEW_ORDER_PACK_SHIFT: u16 = 2;
/// Low-byte mask of the view order indices.
const VIEW_ORDER_LOW_BYTE_MASK: u16 = 0xFF;
/// `temporal_id_start` shift in the temporal/flags byte.
const TEMPORAL_ID_START_SHIFT: u8 = 5;
/// `temporal_id_end` shift in the temporal/flags byte.
const TEMPORAL_ID_END_SHIFT: u8 = 2;
/// Mask of a 3-bit `temporal_id`.
const TEMPORAL_ID_MASK: u8 = 0x07;
/// Wire width of a `temporal_id` (3 bits).
const TEMPORAL_ID_BITS: u32 = 3;
/// `no_sei_nal_unit_present` bit of the temporal/flags byte.
const NO_SEI_MASK: u8 = 0x02;
/// `no_prefix_nal_unit_present` bit of the temporal/flags byte.
const NO_PREFIX_MASK: u8 = 0x01;
/// Shift of `view_association_not_present` in the view-flags byte.
const VIEW_ASSOCIATION_SHIFT: u8 = 7;
/// Shift of `base_view_is_left_eyeview` in the view-flags byte.
const BASE_VIEW_LEFT_EYE_SHIFT: u8 = 6;
/// Shift of `no_sei_nal_unit_present` in the temporal/flags byte.
const NO_SEI_SHIFT: u8 = 1;

/// MVC Extension Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MvcExtensionDescriptor {
    /// Average bit rate in kbit/s (`0` means unspecified).
    pub average_bit_rate: u16,
    /// Maximum bitrate in kbit/s (`0` means unspecified).
    pub maximum_bitrate: u16,
    /// View association not present flag.
    pub view_association_not_present: bool,
    /// Base view is left eye view flag.
    pub base_view_is_left_eyeview: bool,
    /// Minimum view order index.
    pub view_order_index_min: u16,
    /// Maximum view order index.
    pub view_order_index_max: u16,
    /// Temporal ID start (inclusive).
    pub temporal_id_start: u8,
    /// Temporal ID end (inclusive).
    pub temporal_id_end: u8,
    /// No SEI NAL unit present flag.
    pub no_sei_nal_unit_present: bool,
    /// No prefix NAL unit present flag.
    pub no_prefix_nal_unit_present: bool,
    /// `reserved(2)` before `view_order_index_min` (2 bits). Captured on
    /// parse, re-emitted verbatim; defaults to [`RESERVED_DEFAULT`].
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<RESERVED_BITS>,
}

impl MvcExtensionDescriptor {
    /// Build with the bitrates; every flag clear, indices 0 and the
    /// spec-default reserved bits. Set the remaining fields directly.
    #[must_use]
    pub const fn new(average_bit_rate: u16, maximum_bitrate: u16) -> Self {
        Self {
            average_bit_rate,
            maximum_bitrate,
            view_association_not_present: false,
            base_view_is_left_eyeview: false,
            view_order_index_min: 0,
            view_order_index_max: 0,
            temporal_id_start: 0,
            temporal_id_end: 0,
            no_sei_nal_unit_present: false,
            no_prefix_nal_unit_present: false,
            reserved: RESERVED_DEFAULT,
        }
    }
}

impl Default for MvcExtensionDescriptor {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl<'a> Parse<'a> for MvcExtensionDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MvcExtensionDescriptor",
            "unexpected tag for MVC_extension_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "MVC_extension_descriptor length must equal 8",
            });
        }
        let average_bit_rate = u16::from_be_bytes([body[0], body[1]]);
        let maximum_bitrate = u16::from_be_bytes([
            body[MAXIMUM_BITRATE_OFFSET],
            body[MAXIMUM_BITRATE_OFFSET + 1],
        ]);
        let b4 = body[VIEW_FLAGS_OFFSET];
        let b5 = body[VIEW_ORDER_CONTINUATION_OFFSET];
        let b6 = body[VIEW_ORDER_MAX_HI_OFFSET];
        let b7 = body[TEMPORAL_FLAGS_OFFSET];
        let view_association_not_present = (b4 & VIEW_ASSOCIATION_MASK) != 0;
        let base_view_is_left_eyeview = (b4 & BASE_VIEW_LEFT_EYE_MASK) != 0;
        // b4 bits 5-4: reserved
        let view_order_index_min = ((u16::from(b4) & u16::from(VIEW_ORDER_MIN_HI_MASK))
            << VIEW_ORDER_MIN_HI_SHIFT)
            | ((u16::from(b5) >> VIEW_ORDER_MIN_LO_SHIFT) & u16::from(VIEW_ORDER_MIN_LO_MASK));
        let view_order_index_max = ((u16::from(b5) & u16::from(VIEW_ORDER_MAX_HI_MASK))
            << VIEW_ORDER_MAX_HI_SHIFT)
            | u16::from(b6);
        let temporal_id_start = (b7 >> TEMPORAL_ID_START_SHIFT) & TEMPORAL_ID_MASK;
        let temporal_id_end = (b7 >> TEMPORAL_ID_END_SHIFT) & TEMPORAL_ID_MASK;
        let no_sei_nal_unit_present = (b7 & NO_SEI_MASK) != 0;
        let no_prefix_nal_unit_present = (b7 & NO_PREFIX_MASK) != 0;
        Ok(Self {
            average_bit_rate,
            maximum_bitrate,
            view_association_not_present,
            base_view_is_left_eyeview,
            view_order_index_min,
            view_order_index_max,
            temporal_id_start,
            temporal_id_end,
            no_sei_nal_unit_present,
            no_prefix_nal_unit_present,
            reserved: ReservedOnes::from_bits(u64::from((b4 >> RESERVED_SHIFT) & RESERVED_MASK)),
        })
    }
}

impl Serialize for MvcExtensionDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN as usize
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN..HEADER_LEN + size_of::<u16>()]
            .copy_from_slice(&self.average_bit_rate.to_be_bytes());
        buf[HEADER_LEN + MAXIMUM_BITRATE_OFFSET
            ..HEADER_LEN + MAXIMUM_BITRATE_OFFSET + size_of::<u16>()]
            .copy_from_slice(&self.maximum_bitrate.to_be_bytes());
        // Over-wide values used to be masked down silently (audit r03-W3
        // class).
        let vmin = broadcast_common::len::fit_bits(
            u64::from(self.view_order_index_min),
            VIEW_ORDER_INDEX_BITS,
            "view_order_index_min",
        )? as u16;
        let vmax = broadcast_common::len::fit_bits(
            u64::from(self.view_order_index_max),
            VIEW_ORDER_INDEX_BITS,
            "view_order_index_max",
        )? as u16;
        broadcast_common::len::fit_bits(
            u64::from(self.temporal_id_start),
            TEMPORAL_ID_BITS,
            "temporal_id_start",
        )?;
        broadcast_common::len::fit_bits(
            u64::from(self.temporal_id_end),
            TEMPORAL_ID_BITS,
            "temporal_id_end",
        )?;
        buf[HEADER_LEN + VIEW_FLAGS_OFFSET] = ((self.view_association_not_present as u8)
            << VIEW_ASSOCIATION_SHIFT)
            | ((self.base_view_is_left_eyeview as u8) << BASE_VIEW_LEFT_EYE_SHIFT)
            | ((self.reserved.get() as u8) << RESERVED_SHIFT)
            | ((vmin >> VIEW_ORDER_HI_SHIFT) as u8 & VIEW_ORDER_MIN_HI_MASK);
        buf[HEADER_LEN + VIEW_ORDER_CONTINUATION_OFFSET] =
            (((vmin & VIEW_ORDER_MIN_LO_SERIALIZE_MASK) << VIEW_ORDER_PACK_SHIFT) as u8)
                | ((vmax >> VIEW_ORDER_MAX_HI_SHIFT) as u8 & VIEW_ORDER_MAX_HI_MASK);
        buf[HEADER_LEN + VIEW_ORDER_MAX_HI_OFFSET] = (vmax & VIEW_ORDER_LOW_BYTE_MASK) as u8;
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
            | (self.no_prefix_nal_unit_present as u8);
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for MvcExtensionDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MVC_EXTENSION";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse_extracts_fields() {
        let bytes = [
            TAG, 8, 0x00, 0x64, // average_bit_rate = 100
            0x00, 0xC8, // maximum_bitrate = 200
            0x0F, // VA_not_present=0, left_eye=0, reserved=0, vmin_high=15
            0xFC, // vmin_low=63, vmax_high=0
            0xE8, // vmax_low=232
            0x4A, // temporal_id_start=2, temporal_id_end=2, no_sei=1, no_prefix=0
        ];
        let d = MvcExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.average_bit_rate, 100);
        assert_eq!(d.maximum_bitrate, 200);
        assert!(!d.view_association_not_present);
        assert!(!d.base_view_is_left_eyeview);
        assert_eq!(d.view_order_index_min, (15 << 6) | 63);
        assert_eq!(d.view_order_index_max, 232);
        assert_eq!(d.temporal_id_start, 2);
        assert_eq!(d.temporal_id_end, 2);
        assert!(d.no_sei_nal_unit_present);
        assert!(!d.no_prefix_nal_unit_present);
    }

    #[test]
    fn parse_flags() {
        let bytes = [
            TAG, 8, 0, 0, 0, 0, 0xC0, // VA_not_present=1, left_eye=1
            0, 0, 0x03, // temporal_start=0, temporal_end=0, no_sei=1, no_prefix=1
        ];
        let d = MvcExtensionDescriptor::parse(&bytes).unwrap();
        assert!(d.view_association_not_present);
        assert!(d.base_view_is_left_eyeview);
        assert!(d.no_sei_nal_unit_present);
        assert!(d.no_prefix_nal_unit_present);
    }

    #[test]
    fn parse_max_10bit_fields() {
        let bytes = [
            TAG, 8, 0, 0, 0, 0, 0x0F, 0xFF, // vmin = 1023
            0xFF, 0x00, // vmax = 1023
        ];
        let d = MvcExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.view_order_index_min, 1023);
        assert_eq!(d.view_order_index_max, 1023);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MvcExtensionDescriptor::parse(&[0x02, 8, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = MvcExtensionDescriptor::parse(&[TAG, 7, 0, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let mut d = MvcExtensionDescriptor::new(5000, 10000);
        d.base_view_is_left_eyeview = true;
        d.view_order_index_min = 512;
        d.view_order_index_max = 256;
        d.temporal_id_start = 3;
        d.temporal_id_end = 5;
        d.no_sei_nal_unit_present = true;
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MvcExtensionDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = MvcExtensionDescriptor::default();
        let mut tiny = vec![0u8; 5];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall(..)));
    }

    /// H.222.0 Table 2-100. The reserved(2) is preserved verbatim (replaces the
    /// earlier wipe/normalize tests).
    #[test]
    fn mvc_extension_reserved_preserved() {
        for r in 0u8..=3 {
            let bytes = [
                TAG,
                8,
                0x00,
                0x64,
                0x00,
                0xC8,
                (r << 4) | 0x0F,
                0xFC,
                0xE8,
                0x4A,
            ];
            let d = MvcExtensionDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved, ReservedOnes::from_bits(u64::from(r)));
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn mvc_extension_default_is_spec_value() {
        let mut d = MvcExtensionDescriptor::new(0x64, 0xC8);
        d.view_order_index_min = (15 << 6) | 63;
        d.view_order_index_max = 232;
        d.temporal_id_start = 2;
        d.temporal_id_end = 2;
        d.no_sei_nal_unit_present = true;
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out,
            [TAG, 8, 0x00, 0x64, 0x00, 0xC8, 0x3F, 0xFC, 0xE8, 0x4A]
        );
    }

    /// The field is 2 bits wide, so a wider value is unrepresentable and cannot
    /// spill into the `view_order_index_min` bits below it.
    #[test]
    fn reserved_cannot_hold_more_than_two_bits() {
        let d = MvcExtensionDescriptor {
            reserved: ReservedOnes::from_bits(0xFF),
            ..MvcExtensionDescriptor::new(0x0064, 0x00C8)
        };
        assert_eq!(d.reserved.get(), 0x03);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out[6] >> 4, 0x03);
    }
}
