//! avail_descriptor() — ANSI/SCTE 35 2023r1 §10.3.1, Table 18 (tag 0x00).
//!
//! Carries a 32-bit `provider_avail_id` for an avail (analog cue-tone
//! replacement). Intended for use with a splice_insert() command.

use super::header::{self, CUEI, HEADER_LEN};
use crate::error::{Error, Result};
use crate::traits::SpliceDescriptorDef;
use broadcast_common::{Parse, Serialize};

/// `splice_descriptor_tag` for avail_descriptor (§10.1, Table 16).
pub const TAG: u8 = 0x00;

/// avail_descriptor() — §10.3.1, Table 18.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AvailDescriptor {
    /// 32-bit `identifier` (shall be "CUEI" = 0x43554549).
    pub identifier: u32,
    /// 32-bit `provider_avail_id`.
    pub provider_avail_id: u32,
}

impl Default for AvailDescriptor {
    fn default() -> Self {
        Self {
            identifier: CUEI,
            provider_avail_id: 0,
        }
    }
}

impl<'a> Parse<'a> for AvailDescriptor {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (identifier, body) = header::descriptor_body(bytes, TAG, "avail_descriptor")?;
        if body.len() < 4 {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + 4,
                have: bytes.len(),
                what: "avail_descriptor provider_avail_id",
            });
        }
        let provider_avail_id =
            u32::from_be_bytes(*body.first_chunk::<4>().ok_or(Error::BufferTooShort {
                need: HEADER_LEN + 4,
                have: bytes.len(),
                what: "avail_descriptor provider_avail_id",
            })?);
        Ok(Self {
            identifier,
            provider_avail_id,
        })
    }
}

impl Serialize for AvailDescriptor {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        header::write_header(buf, TAG, self.identifier, 4);
        buf[HEADER_LEN..need].copy_from_slice(&self.provider_avail_id.to_be_bytes());
        Ok(need)
    }
}

impl<'a> SpliceDescriptorDef<'a> for AvailDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AVAIL";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let d = AvailDescriptor {
            identifier: CUEI,
            provider_avail_id: 0xABCD_1234,
        };
        let bytes = d.to_bytes();
        assert_eq!(bytes[0], TAG);
        assert_eq!(bytes[1], 0x08);
        let back = AvailDescriptor::parse(&bytes).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn rejects_wrong_tag() {
        let bytes = [0x01, 0x08, 0x43, 0x55, 0x45, 0x49, 0, 0, 0, 0];
        assert!(matches!(
            AvailDescriptor::parse(&bytes).unwrap_err(),
            Error::UnexpectedDescriptorTag { tag: 0x01, .. }
        ));
    }
}
