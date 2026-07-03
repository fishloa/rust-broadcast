//! MVC Extension Descriptor — ISO/IEC 13818-1 §2.6.78, Table 2-100 (tag 0x31).
//!
//! Carries MVC (Multiview Video Coding, H.264 Annex H) extension parameters
//! for a video elementary stream.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for MVC_extension_descriptor.
pub const TAG: u8 = 0x31;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 8;

/// MVC Extension Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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
        let maximum_bitrate = u16::from_be_bytes([body[2], body[3]]);
        let b4 = body[4];
        let b5 = body[5];
        let b6 = body[6];
        let b7 = body[7];
        let view_association_not_present = (b4 & 0x80) != 0;
        let base_view_is_left_eyeview = (b4 & 0x40) != 0;
        // b4 bits 5-4: reserved
        let view_order_index_min = ((b4 as u16 & 0x0F) << 6) | ((b5 as u16 >> 2) & 0x3F);
        let view_order_index_max = ((b5 as u16 & 0x03) << 8) | (b6 as u16);
        let temporal_id_start = (b7 >> 5) & 0x07;
        let temporal_id_end = (b7 >> 2) & 0x07;
        let no_sei_nal_unit_present = (b7 & 0x02) != 0;
        let no_prefix_nal_unit_present = (b7 & 0x01) != 0;
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
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN..HEADER_LEN + 2].copy_from_slice(&self.average_bit_rate.to_be_bytes());
        buf[HEADER_LEN + 2..HEADER_LEN + 4].copy_from_slice(&self.maximum_bitrate.to_be_bytes());
        let vmin = self.view_order_index_min & 0x03FF;
        let vmax = self.view_order_index_max & 0x03FF;
        buf[HEADER_LEN + 4] = ((self.view_association_not_present as u8) << 7)
            | ((self.base_view_is_left_eyeview as u8) << 6)
            | ((vmin >> 6) as u8 & 0x0F);
        buf[HEADER_LEN + 5] = (((vmin & 0x3F) << 2) as u8) | ((vmax >> 8) as u8 & 0x03);
        buf[HEADER_LEN + 6] = (vmax & 0xFF) as u8;
        buf[HEADER_LEN + 7] = ((self.temporal_id_start & 0x07) << 5)
            | ((self.temporal_id_end & 0x07) << 2)
            | ((self.no_sei_nal_unit_present as u8) << 1)
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
        let d = MvcExtensionDescriptor {
            average_bit_rate: 5000,
            maximum_bitrate: 10000,
            view_association_not_present: false,
            base_view_is_left_eyeview: true,
            view_order_index_min: 512,
            view_order_index_max: 256,
            temporal_id_start: 3,
            temporal_id_end: 5,
            no_sei_nal_unit_present: true,
            no_prefix_nal_unit_present: false,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MvcExtensionDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_round_trip_reserved_preserved() {
        // Reserved bits in byte 4 are not part of any field that the struct
        // can round-trip; the serializer writes zero for those bits.
        // This test confirms we parse and re-serialize deterministically
        // with the reserved bits wiped.
        let bytes_with_reserved = [
            TAG, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0x30, // reserved bits non-zero (bits 5-4 = 11)
        ];
        let d = MvcExtensionDescriptor::parse(&bytes_with_reserved).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // Re-parse the serialized output and verify it's stable
        let reparsed = MvcExtensionDescriptor::parse(&buf).unwrap();
        let mut buf2 = vec![0u8; reparsed.serialized_len()];
        reparsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = MvcExtensionDescriptor {
            average_bit_rate: 0,
            maximum_bitrate: 0,
            view_association_not_present: false,
            base_view_is_left_eyeview: false,
            view_order_index_min: 0,
            view_order_index_max: 0,
            temporal_id_start: 0,
            temporal_id_end: 0,
            no_sei_nal_unit_present: false,
            no_prefix_nal_unit_present: false,
        };
        let mut tiny = vec![0u8; 5];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
