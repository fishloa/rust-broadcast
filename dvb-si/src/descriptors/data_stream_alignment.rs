//! Data Stream Alignment Descriptor — ISO/IEC 13818-1 §2.6.14 (tag 0x06).
//!
//! Indicates the alignment of data within PES packets.
//! Alignment type values per ISO/IEC 13818-1 Table 2-53.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for data_stream_alignment_descriptor.
pub const TAG: u8 = 0x06;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 1;

/// Alignment type values per ISO/IEC 13818-1 Table 2-53.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AlignmentType {
    /// Slice or video access unit aligned at PES packet start.
    SliceOrVideoAccessUnit = 0x01,
    /// Video access unit aligned at PES packet start.
    VideoAccessUnit = 0x02,
    /// GOP or SEQ aligned at PES packet start.
    GopOrSeq = 0x03,
    /// SEQ aligned at PES packet start.
    Seq = 0x04,
}

impl AlignmentType {
    /// Construct from a raw byte value.
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::SliceOrVideoAccessUnit),
            0x02 => Some(Self::VideoAccessUnit),
            0x03 => Some(Self::GopOrSeq),
            0x04 => Some(Self::Seq),
            _ => None,
        }
    }

    /// Return the raw byte value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Returns a human-readable name, or `"reserved"` for unrecognised values.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::SliceOrVideoAccessUnit => "Slice or video access unit",
            Self::VideoAccessUnit => "Video access unit",
            Self::GopOrSeq => "GOP or SEQ",
            Self::Seq => "SEQ",
        }
    }
}

/// Data Stream Alignment Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataStreamAlignmentDescriptor {
    /// Alignment type (see [`AlignmentType`]).
    pub alignment_type: u8,
}

impl DataStreamAlignmentDescriptor {
    /// Returns the decoded alignment type, or `None` for reserved values.
    #[must_use]
    pub fn alignment(&self) -> Option<AlignmentType> {
        AlignmentType::from_u8(self.alignment_type)
    }
}

impl<'a> Parse<'a> for DataStreamAlignmentDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DataStreamAlignmentDescriptor",
            "unexpected tag for data_stream_alignment_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "data_stream_alignment_descriptor length must equal 1",
            });
        }
        Ok(Self {
            alignment_type: body[0],
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
impl<'a> crate::traits::DescriptorDef<'a> for DataStreamAlignmentDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DATA_STREAM_ALIGNMENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_slice_or_video_access_unit() {
        let bytes = [TAG, 1, 0x01];
        let d = DataStreamAlignmentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.alignment_type, 0x01);
        assert_eq!(d.alignment(), Some(AlignmentType::SliceOrVideoAccessUnit));
    }

    #[test]
    fn parse_video_access_unit() {
        let bytes = [TAG, 1, 0x02];
        let d = DataStreamAlignmentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.alignment_type, 0x02);
        assert_eq!(d.alignment(), Some(AlignmentType::VideoAccessUnit));
    }

    #[test]
    fn parse_gop_or_seq() {
        let bytes = [TAG, 1, 0x03];
        let d = DataStreamAlignmentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.alignment_type, 0x03);
        assert_eq!(d.alignment(), Some(AlignmentType::GopOrSeq));
    }

    #[test]
    fn parse_seq() {
        let bytes = [TAG, 1, 0x04];
        let d = DataStreamAlignmentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.alignment_type, 0x04);
        assert_eq!(d.alignment(), Some(AlignmentType::Seq));
    }

    #[test]
    fn alignment_type_conversion() {
        assert_eq!(
            AlignmentType::from_u8(0x01).unwrap(),
            AlignmentType::SliceOrVideoAccessUnit
        );
        assert_eq!(
            AlignmentType::from_u8(0x02).unwrap(),
            AlignmentType::VideoAccessUnit
        );
        assert_eq!(
            AlignmentType::from_u8(0x03).unwrap(),
            AlignmentType::GopOrSeq
        );
        assert_eq!(AlignmentType::from_u8(0x04).unwrap(), AlignmentType::Seq);
        assert_eq!(AlignmentType::from_u8(0x00), None);
        assert_eq!(AlignmentType::from_u8(0x05), None);
        assert_eq!(AlignmentType::from_u8(0xFF), None);
    }

    #[test]
    fn alignment_type_round_trip() {
        for v in [0x01, 0x02, 0x03, 0x04] {
            let at = AlignmentType::from_u8(v).unwrap();
            assert_eq!(at.to_u8(), v, "round-trip failed for {v:#04x}");
        }
    }

    #[test]
    fn alignment_type_name() {
        assert_eq!(
            AlignmentType::SliceOrVideoAccessUnit.name(),
            "Slice or video access unit"
        );
        assert_eq!(AlignmentType::VideoAccessUnit.name(), "Video access unit");
        assert_eq!(AlignmentType::GopOrSeq.name(), "GOP or SEQ");
        assert_eq!(AlignmentType::Seq.name(), "SEQ");
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
        assert_eq!(d.serialized_len() - 2, 1);
    }
}
