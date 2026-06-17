//! AVC Video Descriptor — ISO/IEC 13818-1 §2.6.64 (tag 0x28).
//!
//! Describes the profile, level, and constraint flags for an AVC/H.264
//! video elementary stream.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for AVC_video_descriptor.
pub const TAG: u8 = 0x28;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 4;

/// AVC Video Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AvcVideoDescriptor {
    /// AVC profile_idc.
    pub profile_idc: u8,
    /// Constraint set 0 flag.
    pub constraint_set0_flag: bool,
    /// Constraint set 1 flag.
    pub constraint_set1_flag: bool,
    /// Constraint set 2 flag.
    pub constraint_set2_flag: bool,
    /// Constraint set 3 flag.
    pub constraint_set3_flag: bool,
    /// Constraint set 4 flag.
    pub constraint_set4_flag: bool,
    /// Constraint set 5 flag.
    pub constraint_set5_flag: bool,
    /// AVC compatible flags (2 bits).
    pub avc_compatible_flags: u8,
    /// AVC level_idc.
    pub level_idc: u8,
    /// AVC still present flag.
    pub avc_still_present: bool,
    /// AVC 24 hour picture flag.
    pub avc_24_hour_picture_flag: bool,
    /// Frame packing SEI not present flag.
    pub frame_packing_sei_not_present_flag: bool,
}

impl<'a> Parse<'a> for AvcVideoDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AvcVideoDescriptor",
            "unexpected tag for AVC_video_descriptor",
        )?;
        if body.len() < (BODY_LEN as usize) {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "AVC_video_descriptor too short",
            });
        }
        let b0 = body[0]; // profile_idc
        let b1 = body[1]; // constraint_set0_flag..avc_compatible_flags
        let b2 = body[2]; // level_idc
        let b3 = body[3]; // avc_still_present..reserved

        Ok(Self {
            profile_idc: b0,
            constraint_set0_flag: (b1 & 0x80) != 0,
            constraint_set1_flag: (b1 & 0x40) != 0,
            constraint_set2_flag: (b1 & 0x20) != 0,
            constraint_set3_flag: (b1 & 0x10) != 0,
            constraint_set4_flag: (b1 & 0x08) != 0,
            constraint_set5_flag: (b1 & 0x04) != 0,
            avc_compatible_flags: b1 & 0x03,
            level_idc: b2,
            avc_still_present: (b3 & 0x80) != 0,
            avc_24_hour_picture_flag: (b3 & 0x40) != 0,
            frame_packing_sei_not_present_flag: (b3 & 0x20) != 0,
        })
    }
}

impl Serialize for AvcVideoDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
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
        buf[HEADER_LEN] = self.profile_idc;
        buf[HEADER_LEN + 1] = ((self.constraint_set0_flag as u8) << 7)
            | ((self.constraint_set1_flag as u8) << 6)
            | ((self.constraint_set2_flag as u8) << 5)
            | ((self.constraint_set3_flag as u8) << 4)
            | ((self.constraint_set4_flag as u8) << 3)
            | ((self.constraint_set5_flag as u8) << 2)
            | (self.avc_compatible_flags & 0x03);
        buf[HEADER_LEN + 2] = self.level_idc;
        buf[HEADER_LEN + 3] = ((self.avc_still_present as u8) << 7)
            | ((self.avc_24_hour_picture_flag as u8) << 6)
            | ((self.frame_packing_sei_not_present_flag as u8) << 5);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for AvcVideoDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AVC_VIDEO";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let orig = AvcVideoDescriptor {
            profile_idc: 0x4D,
            constraint_set0_flag: true,
            constraint_set1_flag: false,
            constraint_set2_flag: false,
            constraint_set3_flag: true,
            constraint_set4_flag: false,
            constraint_set5_flag: true,
            avc_compatible_flags: 0x03,
            level_idc: 0x29,
            avc_still_present: false,
            avc_24_hour_picture_flag: true,
            frame_packing_sei_not_present_flag: false,
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = AvcVideoDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = AvcVideoDescriptor::parse(&[0x02, 4, 0x00, 0x00, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        // length=2 with a present 2-byte body (< BODY_LEN=4) hits the
        // descriptor's own length check → InvalidDescriptor.
        let err = AvcVideoDescriptor::parse(&[TAG, 2, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }
}
