//! Enhanced AC-3 Descriptor — ETSI EN 300 468 Annex D (tag 0x7A).
//!
//! Same flags-and-fields layout as AC-3 plus two extra bits
//! (mixinfoexists, substream ids). A simpler subset is modelled here; the
//! descriptor body beyond the documented flags is retained as raw bytes.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for Enhanced AC-3 (E-AC-3, Dolby Digital Plus).
pub const TAG: u8 = 0x7A;
const HEADER_LEN: usize = 2;

/// Enhanced AC-3 Descriptor. Body is opaque for this subset — full Annex D
/// parsing is deferred; we preserve the bytes verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EnhancedAc3Descriptor<'a> {
    /// Raw payload (everything after the 2-byte header).
    pub body: &'a [u8],
}

impl<'a> Parse<'a> for EnhancedAc3Descriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "EnhancedAc3Descriptor header",
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
        Ok(Self {
            body: &bytes[HEADER_LEN..end],
        })
    }
}

impl Serialize for EnhancedAc3Descriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body.len()
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
        buf[1] = self.body.len() as u8;
        buf[HEADER_LEN..len].copy_from_slice(self.body);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for EnhancedAc3Descriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        self.body.len() as u8
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
    fn parse_extracts_body() {
        let bytes = [TAG, 3, 0xAA, 0xBB, 0xCC];
        let d = EnhancedAc3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.body, &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            EnhancedAc3Descriptor::parse(&[0x6A, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x6A, .. }
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
            body: &[0x01, 0x02, 0x03],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(EnhancedAc3Descriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn empty_body_valid() {
        let d = EnhancedAc3Descriptor::parse(&[TAG, 0]).unwrap();
        assert_eq!(d.body, &[] as &[u8]);
    }
}
