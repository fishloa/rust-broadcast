//! Compressed Module Descriptor — ETSI EN 301 192 V1.7.1 §10.2.11, Table 59
//! (data-carousel tag 0x09). PDF p.68, transcribed in
//! `docs/descriptors/en_301_192/59-*.md`, verified against the rendered page.
//!
//! Its presence says the module has the "zlib" structure of IETF RFC 1950.
//! Fixed 5-byte body: `compression_method`(8) and `original_size`(32).
//! (The zlib stream itself is the *module* payload, not part of the
//! descriptor — see [`crate::carousel::biop::message::decompress_zlib`]
//! behind the `flate2` feature.)

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for compressed_module_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x09;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 5;

/// Compressed Module Descriptor (carousel tag 0x09).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompressedModuleDescriptor {
    /// `compression_method` — follows the zlib CM definition of IETF RFC 1950
    /// (an external codec code: 8 = deflate).
    pub compression_method: u8,
    /// `original_size` — module size in bytes before compression.
    pub original_size: u32,
}

impl<'a> Parse<'a> for CompressedModuleDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "CompressedModuleDescriptor",
            "unexpected tag for compressed_module_descriptor",
        )?;
        let b: [u8; BODY_LEN] = body.try_into().map_err(|_| Error::InvalidDescriptor {
            tag: TAG,
            reason: "compressed_module_descriptor body must be 5 bytes",
        })?;
        Ok(Self {
            compression_method: b[0],
            original_size: u32::from_be_bytes([b[1], b[2], b[3], b[4]]),
        })
    }
}

impl Serialize for CompressedModuleDescriptor {
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
        buf[2] = self.compression_method;
        buf[3..7].copy_from_slice(&self.original_size.to_be_bytes());
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for CompressedModuleDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "COMPRESSED_MODULE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip_real_capture_bytes() {
        // Verbatim from the France TNT HbbTV capture's DII moduleInfo userInfo
        // (`09 05 78 00 00 eb 2b`).
        let bytes = [TAG, 5, 0x78, 0x00, 0x00, 0xEB, 0x2B];
        let d = CompressedModuleDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.compression_method, 0x78);
        assert_eq!(d.original_size, 0xEB2B);
        let mut out = [0u8; 7];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn wrong_length_rejected() {
        assert!(CompressedModuleDescriptor::parse(&[TAG, 4, 8, 0, 0, 0]).is_err());
        assert!(CompressedModuleDescriptor::parse(&[TAG, 6, 8, 0, 0, 0, 0, 0]).is_err());
    }
}
