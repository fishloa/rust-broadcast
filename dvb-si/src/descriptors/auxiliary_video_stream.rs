//! Auxiliary Video Stream Descriptor — ISO/IEC 13818-1 §2.6.74, Table 2-98 (tag 0x2F).
//!
//! Carries an aux_video_codedstreamtype byte followed by an si_rbsp()
//! opaque payload from ISO/IEC 23002-3.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for Auxiliary_video_stream_descriptor.
pub const TAG: u8 = 0x2F;
const HEADER_LEN: usize = 2;
const FIXED_LEN: usize = 1;

/// Auxiliary Video Stream Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AuxiliaryVideoStreamDescriptor<'a> {
    /// Auxiliary video coded stream type.
    pub aux_video_codedstreamtype: u8,
    /// si_rbsp() from ISO/IEC 23002-3 carried as opaque bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub si_rbsp: &'a [u8],
}

impl<'a> Parse<'a> for AuxiliaryVideoStreamDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AuxiliaryVideoStreamDescriptor",
            "unexpected tag for Auxiliary_video_stream_descriptor",
        )?;
        if body.len() < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "Auxiliary_video_stream_descriptor length too short (need >= 1)",
            });
        }
        Ok(Self {
            aux_video_codedstreamtype: body[0],
            si_rbsp: &body[FIXED_LEN..],
        })
    }
}

impl Serialize for AuxiliaryVideoStreamDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.si_rbsp.len()
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
        buf[1] = (len - HEADER_LEN) as u8;
        buf[HEADER_LEN] = self.aux_video_codedstreamtype;
        buf[HEADER_LEN + FIXED_LEN..len].copy_from_slice(self.si_rbsp);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for AuxiliaryVideoStreamDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AUXILIARY_VIDEO_STREAM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal() {
        let bytes = [TAG, 1, 0x42];
        let d = AuxiliaryVideoStreamDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.aux_video_codedstreamtype, 0x42);
        assert!(d.si_rbsp.is_empty());
    }

    #[test]
    fn parse_with_rbsp() {
        let bytes = [TAG, 3, 0x01, 0xAA, 0xBB];
        let d = AuxiliaryVideoStreamDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.aux_video_codedstreamtype, 0x01);
        assert_eq!(d.si_rbsp, &[0xAA, 0xBB]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = AuxiliaryVideoStreamDescriptor::parse(&[0x02, 1, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        let err = AuxiliaryVideoStreamDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = AuxiliaryVideoStreamDescriptor {
            aux_video_codedstreamtype: 0x07,
            si_rbsp: &[0xDE, 0xAD, 0xBE, 0xEF],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = AuxiliaryVideoStreamDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = AuxiliaryVideoStreamDescriptor {
            aux_video_codedstreamtype: 0,
            si_rbsp: &[],
        };
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
