//! Data Stream Alignment Descriptor — ISO/IEC 13818-1 §2.6.14 (tag 0x06).
//!
//! Indicates the alignment of video and audio data within the PES packets.

use num_enum::TryFromPrimitive;

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for data_stream_alignment_descriptor.
pub const TAG: u8 = 0x06;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 1;

/// Alignment type values per ISO/IEC 13818-1 Table 2-39.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum AlignmentType {
    /// Video access units start at the beginning of a PES packet.
    VideoAccessUnit = 0x01,
    /// Audio access units start at the beginning of a PES packet.
    AudioAccessUnit = 0x02,
    /// Video and audio access units start at the beginning of a PES packet.
    VideoAndAudioAccessUnit = 0x03,
    /// Reserved for future use.
    Reserved = 0x04,
}

/// Data Stream Alignment Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataStreamAlignmentDescriptor {
    /// Alignment type (see [`AlignmentType`]).
    pub alignment_type: u8,
}

impl<'a> Parse<'a> for DataStreamAlignmentDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + BODY_LEN as usize {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + BODY_LEN as usize,
                have: bytes.len(),
                what: "DataStreamAlignmentDescriptor",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for data_stream_alignment_descriptor",
            });
        }
        if bytes[1] != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "data_stream_alignment_descriptor length must equal 1",
            });
        }
        Ok(Self {
            alignment_type: bytes[2],
        })
    }
}

impl Serialize for DataStreamAlignmentDescriptor {
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
        buf[2] = self.alignment_type;
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for DataStreamAlignmentDescriptor {
    const TAG: u8 = TAG;

    fn descriptor_length(&self) -> u8 {
        BODY_LEN
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for DataStreamAlignmentDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DATA_STREAM_ALIGNMENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_video_access_unit() {
        let bytes = [TAG, 1, 0x01];
        let d = DataStreamAlignmentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.alignment_type, 0x01);
    }

    #[test]
    fn parse_video_and_audio() {
        let bytes = [TAG, 1, 0x03];
        let d = DataStreamAlignmentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.alignment_type, 0x03);
    }

    #[test]
    fn alignment_type_conversion() {
        assert_eq!(
            AlignmentType::try_from(0x01).unwrap(),
            AlignmentType::VideoAccessUnit
        );
        assert_eq!(
            AlignmentType::try_from(0x02).unwrap(),
            AlignmentType::AudioAccessUnit
        );
        assert_eq!(
            AlignmentType::try_from(0x03).unwrap(),
            AlignmentType::VideoAndAudioAccessUnit
        );
        assert_eq!(
            AlignmentType::try_from(0x04).unwrap(),
            AlignmentType::Reserved
        );
        assert!(AlignmentType::try_from(0xFF).is_err());
    }

    #[test]
    fn exhaustive_byte_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = AlignmentType::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 4, "expected 4 matched variants");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = DataStreamAlignmentDescriptor::parse(&[0x07, 1, 0x01]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x07, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = DataStreamAlignmentDescriptor::parse(&[TAG, 2, 0x01, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = DataStreamAlignmentDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = DataStreamAlignmentDescriptor {
            alignment_type: 0x03,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = DataStreamAlignmentDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = DataStreamAlignmentDescriptor {
            alignment_type: 0x02,
        };
        assert_eq!(d.descriptor_length(), 1);
    }
}
