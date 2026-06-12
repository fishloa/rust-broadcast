//! Data Stream Alignment Descriptor — ISO/IEC 13818-1 §2.6.10 (tag 0x06).
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
    /// 0x01 — Slice, or video access unit aligned at PES packet start.
    SliceOrVideoAccessUnit,
    /// 0x02 — Video access unit aligned at PES packet start.
    VideoAccessUnit,
    /// 0x03 — GOP, or SEQ aligned at PES packet start.
    GopOrSeq,
    /// 0x04 — SEQ aligned at PES packet start.
    Seq,
    /// Reserved/unrecognised wire value (0x00, 0x05..=0xFF), preserved verbatim
    /// for byte-identical round-trip.
    Reserved(u8),
}

impl AlignmentType {
    /// Construct from a raw byte; unknown values are preserved as `Reserved`.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::SliceOrVideoAccessUnit,
            0x02 => Self::VideoAccessUnit,
            0x03 => Self::GopOrSeq,
            0x04 => Self::Seq,
            v => Self::Reserved(v),
        }
    }

    /// Return the raw byte value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::SliceOrVideoAccessUnit => 0x01,
            Self::VideoAccessUnit => 0x02,
            Self::GopOrSeq => 0x03,
            Self::Seq => 0x04,
            Self::Reserved(v) => v,
        }
    }

    /// Returns a human-readable name, or `"reserved"` for unrecognised values.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::SliceOrVideoAccessUnit => "Slice, or video access unit",
            Self::VideoAccessUnit => "Video access unit",
            Self::GopOrSeq => "GOP, or SEQ",
            Self::Seq => "SEQ",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// Data Stream Alignment Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataStreamAlignmentDescriptor {
    /// Alignment type — typed; reserved/unknown bytes are preserved as
    /// [`AlignmentType::Reserved`] for byte-identical round-trip.
    pub alignment_type: AlignmentType,
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
            alignment_type: AlignmentType::from_u8(body[0]),
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
        buf[2] = self.alignment_type.to_u8();
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
        let d = DataStreamAlignmentDescriptor::parse(&[TAG, 1, 0x01]).unwrap();
        assert_eq!(d.alignment_type, AlignmentType::SliceOrVideoAccessUnit);
    }

    #[test]
    fn parse_video_access_unit() {
        let d = DataStreamAlignmentDescriptor::parse(&[TAG, 1, 0x02]).unwrap();
        assert_eq!(d.alignment_type, AlignmentType::VideoAccessUnit);
    }

    #[test]
    fn parse_gop_or_seq() {
        let d = DataStreamAlignmentDescriptor::parse(&[TAG, 1, 0x03]).unwrap();
        assert_eq!(d.alignment_type, AlignmentType::GopOrSeq);
    }

    #[test]
    fn parse_seq() {
        let d = DataStreamAlignmentDescriptor::parse(&[TAG, 1, 0x04]).unwrap();
        assert_eq!(d.alignment_type, AlignmentType::Seq);
    }

    #[test]
    fn parse_reserved_preserves_byte() {
        // 0x00 and 0x05..=0xFF are reserved; the wire byte must round-trip.
        let d = DataStreamAlignmentDescriptor::parse(&[TAG, 1, 0x05]).unwrap();
        assert_eq!(d.alignment_type, AlignmentType::Reserved(0x05));
        assert_eq!(d.alignment_type.name(), "reserved");
    }

    #[test]
    fn alignment_type_conversion() {
        assert_eq!(
            AlignmentType::from_u8(0x01),
            AlignmentType::SliceOrVideoAccessUnit
        );
        assert_eq!(AlignmentType::from_u8(0x04), AlignmentType::Seq);
        assert_eq!(AlignmentType::from_u8(0x00), AlignmentType::Reserved(0x00));
        assert_eq!(AlignmentType::from_u8(0x05), AlignmentType::Reserved(0x05));
        assert_eq!(AlignmentType::from_u8(0xFF), AlignmentType::Reserved(0xFF));
    }

    #[test]
    fn alignment_type_round_trip() {
        for v in [0x00u8, 0x01, 0x02, 0x03, 0x04, 0x05, 0xFF] {
            assert_eq!(
                AlignmentType::from_u8(v).to_u8(),
                v,
                "round-trip failed for {v:#04x}"
            );
        }
    }

    #[test]
    fn alignment_type_name() {
        assert_eq!(
            AlignmentType::SliceOrVideoAccessUnit.name(),
            "Slice, or video access unit"
        );
        assert_eq!(AlignmentType::VideoAccessUnit.name(), "Video access unit");
        assert_eq!(AlignmentType::GopOrSeq.name(), "GOP, or SEQ");
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
            alignment_type: AlignmentType::GopOrSeq,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = DataStreamAlignmentDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = DataStreamAlignmentDescriptor {
            alignment_type: AlignmentType::VideoAccessUnit,
        };
        assert_eq!(d.serialized_len() - 2, 1);
    }
}
