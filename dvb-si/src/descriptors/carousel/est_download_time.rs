//! Estimated Download Time Descriptor — ETSI EN 301 192 V1.7.1 §10.2.8,
//! Table 56 (data-carousel tag 0x07). PDF p.67, transcribed in
//! `docs/descriptors/en_301_192/56-*.md`, verified against the rendered page.
//!
//! `est_download_time`(32): estimated download time of a module or group, in
//! seconds.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for est_download_time_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x07;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 4;

/// Estimated Download Time Descriptor (carousel tag 0x07).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EstDownloadTimeDescriptor {
    /// `est_download_time` — estimated download time in seconds.
    pub est_download_time: u32,
}

impl<'a> Parse<'a> for EstDownloadTimeDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "EstDownloadTimeDescriptor",
            "unexpected tag for est_download_time_descriptor",
        )?;
        let b: [u8; BODY_LEN] = body.try_into().map_err(|_| Error::InvalidDescriptor {
            tag: TAG,
            reason: "est_download_time_descriptor body must be 4 bytes",
        })?;
        Ok(Self {
            est_download_time: u32::from_be_bytes(b),
        })
    }
}

impl Serialize for EstDownloadTimeDescriptor {
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
        buf[2..6].copy_from_slice(&self.est_download_time.to_be_bytes());
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for EstDownloadTimeDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "EST_DOWNLOAD_TIME";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let bytes = [TAG, 4, 0x00, 0x00, 0x01, 0x2C];
        let d = EstDownloadTimeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.est_download_time, 300);
        let mut out = [0u8; 6];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn wrong_length_rejected() {
        assert!(EstDownloadTimeDescriptor::parse(&[TAG, 3, 0, 0, 0]).is_err());
        assert!(EstDownloadTimeDescriptor::parse(&[TAG, 5, 0, 0, 0, 0, 0]).is_err());
    }
}
