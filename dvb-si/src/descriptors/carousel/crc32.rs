//! CRC32 Descriptor — ETSI EN 301 192 V1.7.1 §10.2.6, Table 54 (data-carousel
//! tag 0x05). PDF p.66, transcribed in `docs/descriptors/en_301_192/54-*.md`,
//! verified against the rendered page.
//!
//! The 32-bit `CRC_32` computed over a complete module (ISO/IEC 13818-1
//! Annex B, i.e. the MPEG-2 CRC-32).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for CRC32_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x05;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 4;

/// CRC32 Descriptor (carousel tag 0x05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Crc32Descriptor {
    /// `CRC_32` — the module's CRC.
    pub crc_32: u32,
}

impl Crc32Descriptor {
    /// Does `module` (the complete reassembled module bytes) match `crc_32`?
    #[must_use]
    pub fn matches(&self, module: &[u8]) -> bool {
        broadcast_common::crc32_mpeg2::compute(module) == self.crc_32
    }
}

impl<'a> Parse<'a> for Crc32Descriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Crc32Descriptor",
            "unexpected tag for CRC32_descriptor",
        )?;
        let b: [u8; BODY_LEN] = body.try_into().map_err(|_| Error::InvalidDescriptor {
            tag: TAG,
            reason: "CRC32_descriptor body must be 4 bytes",
        })?;
        Ok(Self {
            crc_32: u32::from_be_bytes(b),
        })
    }
}

impl Serialize for Crc32Descriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, BODY_LEN)?;
        buf[2..6].copy_from_slice(&self.crc_32.to_be_bytes());
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for Crc32Descriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CRC32";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let bytes = [TAG, 4, 0x01, 0x02, 0x03, 0x04];
        let d = Crc32Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.crc_32, 0x0102_0304);
        let mut out = [0u8; 6];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn matches_uses_mpeg2_crc() {
        // CRC-32/MPEG-2 check value of "123456789" is 0x0376E6E7.
        let d = Crc32Descriptor {
            crc_32: 0x0376_E6E7,
        };
        assert!(d.matches(b"123456789"));
        assert!(!d.matches(b"123456780"));
    }

    #[test]
    fn wrong_length_rejected() {
        assert!(Crc32Descriptor::parse(&[TAG, 3, 0, 0, 0]).is_err());
        assert!(Crc32Descriptor::parse(&[TAG, 5, 0, 0, 0, 0, 0]).is_err());
    }
}
