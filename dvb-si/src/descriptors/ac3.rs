//! AC-3 Descriptor — ETSI EN 300 468 Annex D (tag 0x6A).
//!
//! Carried inside PMT's ES_info loop for AC-3 audio components. The layout
//! is a flag byte followed by four optional 1-byte fields and an optional
//! free-form additional_info trailer.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for AC-3 audio.
pub const TAG: u8 = 0x6A;
const HEADER_LEN: usize = 2;

const FLAG_COMPONENT_TYPE: u8 = 0x80;
const FLAG_BSID: u8 = 0x40;
const FLAG_MAINID: u8 = 0x20;
const FLAG_ASVC: u8 = 0x10;

/// AC-3 Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Ac3Descriptor<'a> {
    /// AC-3 component_type (layout per Annex D).
    pub component_type: Option<u8>,
    /// Bit stream identification.
    pub bsid: Option<u8>,
    /// Main audio service id.
    pub mainid: Option<u8>,
    /// Associated service id.
    pub asvc: Option<u8>,
    /// Raw trailing additional_info bytes.
    pub additional_info: &'a [u8],
}

impl<'a> Parse<'a> for Ac3Descriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + 1 {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + 1,
                have: bytes.len(),
                what: "Ac3Descriptor header+flags",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for AC-3 descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "Ac3Descriptor body",
            });
        }
        let flags = bytes[HEADER_LEN];
        let mut pos = HEADER_LEN + 1;
        let mut read_one = |set: bool| -> Result<Option<u8>> {
            if !set {
                return Ok(None);
            }
            if pos >= end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "AC-3 descriptor flags claim more bytes than length permits",
                });
            }
            let b = bytes[pos];
            pos += 1;
            Ok(Some(b))
        };

        let component_type = read_one(flags & FLAG_COMPONENT_TYPE != 0)?;
        let bsid = read_one(flags & FLAG_BSID != 0)?;
        let mainid = read_one(flags & FLAG_MAINID != 0)?;
        let asvc = read_one(flags & FLAG_ASVC != 0)?;
        let additional_info = &bytes[pos..end];
        Ok(Self {
            component_type,
            bsid,
            mainid,
            asvc,
            additional_info,
        })
    }
}

impl Serialize for Ac3Descriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + 1
            + usize::from(self.component_type.is_some())
            + usize::from(self.bsid.is_some())
            + usize::from(self.mainid.is_some())
            + usize::from(self.asvc.is_some())
            + self.additional_info.len()
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
        let mut flags: u8 = 0;
        if self.component_type.is_some() {
            flags |= FLAG_COMPONENT_TYPE;
        }
        if self.bsid.is_some() {
            flags |= FLAG_BSID;
        }
        if self.mainid.is_some() {
            flags |= FLAG_MAINID;
        }
        if self.asvc.is_some() {
            flags |= FLAG_ASVC;
        }
        // The low 4 bits are reserved_future_use and must be set to 1.
        buf[2] = flags | 0x0F;
        let mut pos = 3;
        for b in [self.component_type, self.bsid, self.mainid, self.asvc]
            .into_iter()
            .flatten()
        {
            buf[pos] = b;
            pos += 1;
        }
        buf[pos..pos + self.additional_info.len()].copy_from_slice(self.additional_info);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for Ac3Descriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for Ac3Descriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AC3";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_with_all_fields() {
        let bytes = [
            TAG,
            5,
            FLAG_COMPONENT_TYPE | FLAG_BSID | FLAG_MAINID | FLAG_ASVC,
            0x11,
            0x22,
            0x33,
            0x44,
        ];
        let d = Ac3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_type, Some(0x11));
        assert_eq!(d.bsid, Some(0x22));
        assert_eq!(d.mainid, Some(0x33));
        assert_eq!(d.asvc, Some(0x44));
        assert_eq!(d.additional_info, &[] as &[u8]);
    }

    #[test]
    fn parse_with_only_component_type() {
        let bytes = [TAG, 2, FLAG_COMPONENT_TYPE, 0x07];
        let d = Ac3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_type, Some(0x07));
        assert_eq!(d.bsid, None);
    }

    #[test]
    fn parse_with_additional_info_only() {
        let bytes = [TAG, 3, 0x00, 0xAA, 0xBB];
        let d = Ac3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_type, None);
        assert_eq!(d.additional_info, &[0xAA, 0xBB]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            Ac3Descriptor::parse(&[0x7A, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x7A, .. }
        ));
    }

    #[test]
    fn parse_rejects_flags_past_length() {
        // flags claim component_type but length=1 covers only the flags byte.
        let bytes = [TAG, 1, FLAG_COMPONENT_TYPE];
        assert!(matches!(
            Ac3Descriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = Ac3Descriptor {
            component_type: Some(0x40),
            bsid: Some(8),
            mainid: None,
            asvc: None,
            additional_info: &[0xFE, 0xED],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(Ac3Descriptor::parse(&buf).unwrap(), d);
    }
}
