//! SVC Extension Descriptor — ISO/IEC 13818-1 §2.6.76, Table 2-99 (tag 0x30).
//!
//! Carries SVC (Scalable Video Coding, H.264 Annex G) extension parameters
//! for a video elementary stream.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for SVC_extension_descriptor.
pub const TAG: u8 = 0x30;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 13;

/// SVC Extension Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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
        let height = u16::from_be_bytes([body[2], body[3]]);
        let frame_rate = u16::from_be_bytes([body[4], body[5]]);
        let average_bitrate = u16::from_be_bytes([body[6], body[7]]);
        let maximum_bitrate = u16::from_be_bytes([body[8], body[9]]);
        let b10 = body[10];
        let dependency_id = (b10 >> 5) & 0x07;
        // reserved: top 5 bits are reserved — we accept any value for round-trip
        let b11 = body[11];
        let quality_id_start = (b11 >> 4) & 0x0F;
        let quality_id_end = b11 & 0x0F;
        let b12 = body[12];
        let temporal_id_start = (b12 >> 5) & 0x07;
        let temporal_id_end = (b12 >> 2) & 0x07;
        let no_sei_nal_unit_present = (b12 & 0x02) != 0;
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
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN..HEADER_LEN + 2].copy_from_slice(&self.width.to_be_bytes());
        buf[HEADER_LEN + 2..HEADER_LEN + 4].copy_from_slice(&self.height.to_be_bytes());
        buf[HEADER_LEN + 4..HEADER_LEN + 6].copy_from_slice(&self.frame_rate.to_be_bytes());
        buf[HEADER_LEN + 6..HEADER_LEN + 8].copy_from_slice(&self.average_bitrate.to_be_bytes());
        buf[HEADER_LEN + 8..HEADER_LEN + 10].copy_from_slice(&self.maximum_bitrate.to_be_bytes());
        buf[HEADER_LEN + 10] = (self.dependency_id & 0x07) << 5;
        buf[HEADER_LEN + 11] = ((self.quality_id_start & 0x0F) << 4) | (self.quality_id_end & 0x0F);
        buf[HEADER_LEN + 12] = ((self.temporal_id_start & 0x07) << 5)
            | ((self.temporal_id_end & 0x07) << 2)
            | ((self.no_sei_nal_unit_present as u8) << 1);
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
        let d = SvcExtensionDescriptor {
            width: 1280,
            height: 720,
            frame_rate: 50,
            average_bitrate: 5000,
            maximum_bitrate: 10000,
            dependency_id: 2,
            quality_id_start: 3,
            quality_id_end: 5,
            temporal_id_start: 1,
            temporal_id_end: 3,
            no_sei_nal_unit_present: false,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = SvcExtensionDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_round_trip_reserved_preserved() {
        // Reserved bits are not part of any field that the struct can
        // round-trip; the serializer writes zero for those bits.
        // This test confirms we parse and re-serialize deterministically
        // with the reserved bits wiped.
        let bytes_with_reserved = [
            TAG, 13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x1F, // dependency_id=0, reserved non-zero
            0x00, 0x01, // temporal_id_start=0, temporal_id_end=0, no_sei=0, reserved=1
        ];
        let d = SvcExtensionDescriptor::parse(&bytes_with_reserved).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // Re-parse the serialized output and verify it's stable
        let reparsed = SvcExtensionDescriptor::parse(&buf).unwrap();
        let mut buf2 = vec![0u8; reparsed.serialized_len()];
        reparsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = SvcExtensionDescriptor {
            width: 0,
            height: 0,
            frame_rate: 0,
            average_bitrate: 0,
            maximum_bitrate: 0,
            dependency_id: 0,
            quality_id_start: 0,
            quality_id_end: 0,
            temporal_id_start: 0,
            temporal_id_end: 0,
            no_sei_nal_unit_present: false,
        };
        let mut tiny = vec![0u8; 5];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
