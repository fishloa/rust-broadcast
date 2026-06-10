//! Enhanced AC-3 Descriptor — ETSI EN 300 468 Annex D (tag 0x7A).
//!
//! Carried inside PMT's ES_info loop for Enhanced AC-3 (E-AC-3, Dolby Digital
//! Plus) audio components.  The layout is a single flags byte whose eight bits
//! are all assigned (no reserved nibble, unlike the AC-3 descriptor), followed
//! by up to seven optional 1-byte fields and an optional free-form
//! `additional_info_byte` trailer.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for Enhanced AC-3 (E-AC-3, Dolby Digital Plus).
pub const TAG: u8 = 0x7A;
const HEADER_LEN: usize = 2;

const FLAG_COMPONENT_TYPE: u8 = 0x80;
const FLAG_BSID: u8 = 0x40;
const FLAG_MAINID: u8 = 0x20;
const FLAG_ASVC: u8 = 0x10;
const FLAG_MIXINFO_EXISTS: u8 = 0x08;
const FLAG_SUBSTREAM1: u8 = 0x04;
const FLAG_SUBSTREAM2: u8 = 0x02;
const FLAG_SUBSTREAM3: u8 = 0x01;

/// Enhanced AC-3 Descriptor — EN 300 468 Annex D (tag 0x7A).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct EnhancedAc3Descriptor<'a> {
    /// AC-3 component_type (layout per Annex D).
    pub component_type: Option<u8>,
    /// Bit stream identification.
    pub bsid: Option<u8>,
    /// Main audio service id.
    pub mainid: Option<u8>,
    /// Associated service id.
    pub asvc: Option<u8>,
    /// mixinfoexists flag — indicates whether mixing metadata is present in
    /// the enhanced AC-3 bit stream.
    pub mixinfoexists: bool,
    /// Sub-stream 1 identification.
    pub substream1: Option<u8>,
    /// Sub-stream 2 identification.
    pub substream2: Option<u8>,
    /// Sub-stream 3 identification.
    pub substream3: Option<u8>,
    /// Raw trailing `additional_info_byte` run.
    pub additional_info: &'a [u8],
}

impl<'a> Parse<'a> for EnhancedAc3Descriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + 1 {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + 1,
                have: bytes.len(),
                what: "EnhancedAc3Descriptor header+flags",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for EAC-3 descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "EnhancedAc3Descriptor body",
            });
        }
        let flags = bytes[HEADER_LEN];
        let mixinfoexists = (flags & FLAG_MIXINFO_EXISTS) != 0;
        let mut pos = HEADER_LEN + 1;
        let mut read_one = |set: bool| -> Result<Option<u8>> {
            if !set {
                return Ok(None);
            }
            if pos >= end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "enhanced AC-3 descriptor flags claim more bytes than length permits",
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
        let substream1 = read_one(flags & FLAG_SUBSTREAM1 != 0)?;
        let substream2 = read_one(flags & FLAG_SUBSTREAM2 != 0)?;
        let substream3 = read_one(flags & FLAG_SUBSTREAM3 != 0)?;
        let additional_info = &bytes[pos..end];
        Ok(Self {
            component_type,
            bsid,
            mainid,
            asvc,
            mixinfoexists,
            substream1,
            substream2,
            substream3,
            additional_info,
        })
    }
}

impl Serialize for EnhancedAc3Descriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + 1
            + usize::from(self.component_type.is_some())
            + usize::from(self.bsid.is_some())
            + usize::from(self.mainid.is_some())
            + usize::from(self.asvc.is_some())
            + usize::from(self.substream1.is_some())
            + usize::from(self.substream2.is_some())
            + usize::from(self.substream3.is_some())
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
        if self.mixinfoexists {
            flags |= FLAG_MIXINFO_EXISTS;
        }
        if self.substream1.is_some() {
            flags |= FLAG_SUBSTREAM1;
        }
        if self.substream2.is_some() {
            flags |= FLAG_SUBSTREAM2;
        }
        if self.substream3.is_some() {
            flags |= FLAG_SUBSTREAM3;
        }
        buf[2] = flags;
        let mut pos = 3;
        for b in [
            self.component_type,
            self.bsid,
            self.mainid,
            self.asvc,
            self.substream1,
            self.substream2,
            self.substream3,
        ]
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

impl<'a> Descriptor<'a> for EnhancedAc3Descriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for EnhancedAc3Descriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ENHANCED_AC3";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_with_all_fields() {
        let bytes = [
            TAG,
            8,
            FLAG_COMPONENT_TYPE
                | FLAG_BSID
                | FLAG_MAINID
                | FLAG_ASVC
                | FLAG_MIXINFO_EXISTS
                | FLAG_SUBSTREAM1
                | FLAG_SUBSTREAM2
                | FLAG_SUBSTREAM3,
            0x11,
            0x22,
            0x33,
            0x44,
            0x55,
            0x66,
            0x77,
        ];
        let d = EnhancedAc3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_type, Some(0x11));
        assert_eq!(d.bsid, Some(0x22));
        assert_eq!(d.mainid, Some(0x33));
        assert_eq!(d.asvc, Some(0x44));
        assert!(d.mixinfoexists);
        assert_eq!(d.substream1, Some(0x55));
        assert_eq!(d.substream2, Some(0x66));
        assert_eq!(d.substream3, Some(0x77));
        assert_eq!(d.additional_info, &[] as &[u8]);
    }

    #[test]
    fn parse_with_only_component_type_and_mixinfoexists() {
        let bytes = [TAG, 2, FLAG_COMPONENT_TYPE | FLAG_MIXINFO_EXISTS, 0x07];
        let d = EnhancedAc3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_type, Some(0x07));
        assert!(d.mixinfoexists);
        assert_eq!(d.bsid, None);
        assert_eq!(d.substream1, None);
    }

    #[test]
    fn parse_with_additional_info_only() {
        let bytes = [TAG, 3, 0x00, 0xAA, 0xBB];
        let d = EnhancedAc3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_type, None);
        assert!(!d.mixinfoexists);
        assert_eq!(d.additional_info, &[0xAA, 0xBB]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            EnhancedAc3Descriptor::parse(&[0x6A, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x6A, .. }
        ));
    }

    #[test]
    fn parse_rejects_flags_past_length() {
        let bytes = [TAG, 1, FLAG_COMPONENT_TYPE];
        assert!(matches!(
            EnhancedAc3Descriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            EnhancedAc3Descriptor::parse(&[TAG]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = EnhancedAc3Descriptor {
            component_type: Some(0x40),
            bsid: Some(8),
            mainid: None,
            asvc: None,
            mixinfoexists: true,
            substream1: Some(0xAA),
            substream2: None,
            substream3: None,
            additional_info: &[0xFE, 0xED],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(EnhancedAc3Descriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_round_trip_no_flags() {
        let d = EnhancedAc3Descriptor {
            component_type: None,
            bsid: None,
            mainid: None,
            asvc: None,
            mixinfoexists: false,
            substream1: None,
            substream2: None,
            substream3: None,
            additional_info: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(EnhancedAc3Descriptor::parse(&buf).unwrap(), d);
        assert_eq!(buf, [TAG, 1, 0x00]);
    }
}
