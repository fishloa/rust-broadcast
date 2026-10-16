//! ssu_uri_descriptor — ETSI TS 102 006 §9.5.2.15 Table 38 (UNT tag 0x0D).
//!
//! Indicates the update is available from the internet: `max_holdoff_time`
//! (minutes; the receiver waits a random 0..60x that many seconds before
//! connecting), `min_polling_interval` (hours) and the `uri_char`s of an RFC
//! 3986 URI. A `descriptor_length` of two means no URI is available. UNT
//! common and operational loops (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::DvbText;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for ssu_uri_descriptor.
pub const TAG: u8 = 0x0D;
const HEADER_LEN: usize = 2;
const FIXED_LEN: usize = 2;

/// ssu_uri_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SsuUriDescriptor<'a> {
    /// `max_holdoff_time` — maximum minutes to hold off connection attempts.
    pub max_holdoff_time: u8,
    /// `min_polling_interval` — minimum hours between connection requests;
    /// 0 means no minimum is implied.
    pub min_polling_interval: u8,
    /// `uri_char`s — an RFC 3986 URI; empty when "a URI is not available".
    pub uri: DvbText<'a>,
}

impl<'a> Parse<'a> for SsuUriDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SsuUriDescriptor",
            "unexpected tag for ssu_uri_descriptor",
        )?;
        let (f, uri) = body
            .split_first_chunk::<FIXED_LEN>()
            .ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "ssu_uri_descriptor body shorter than 2 bytes",
            })?;
        Ok(Self {
            max_holdoff_time: f[0],
            min_polling_interval: f[1],
            uri: DvbText::new(uri),
        })
    }
}

impl Serialize for SsuUriDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.uri.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2] = self.max_holdoff_time;
        buf[3] = self.min_polling_interval;
        buf[4..len].copy_from_slice(self.uri.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for SsuUriDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SSU_URI";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let mut bytes = vec![TAG, 0, 5, 24];
        bytes.extend_from_slice(b"http://ssu.example/u");
        bytes[1] = (bytes.len() - 2) as u8;
        let d = SsuUriDescriptor::parse(&bytes).unwrap();
        assert_eq!((d.max_holdoff_time, d.min_polling_interval), (5, 24));
        assert_eq!(d.uri.raw(), b"http://ssu.example/u");
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn length_two_means_no_uri() {
        let bytes = [TAG, 2, 0, 0];
        let d = SsuUriDescriptor::parse(&bytes).unwrap();
        assert!(d.uri.raw().is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert!(SsuUriDescriptor::parse(&[TAG, 1, 0]).is_err());
        assert!(SsuUriDescriptor::parse(&[0x0C, 2, 0, 0]).is_err());
    }
}
