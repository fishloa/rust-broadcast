//! ISP_access_mode_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.16 Table 36
//! (tag 0x14), `access_mode` per Table 37.
//!
//! Gives information about accessing the ISP through alternative media (PSTN
//! etc.) rather than the DVB network. In the dial-up mode at least one
//! `telephone_descriptor` shall be present. INT platform and operational
//! loops (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for ISP_access_mode_descriptor.
pub const TAG: u8 = 0x14;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 1;

/// `access_mode` — ETSI EN 301 192 §8.4.5.16 Table 37.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum IspAccessMode {
    /// 0x00 — not used.
    NotUsed,
    /// 0x01 — dialup.
    Dialup,
    /// 0x02..=0xFF — reserved for future use.
    Reserved(u8),
}

impl IspAccessMode {
    /// Decode from the wire value. Every value maps (lossless).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::NotUsed,
            0x01 => Self::Dialup,
            _ => Self::Reserved(v),
        }
    }

    /// Encode to the wire value. Inverse of [`from_u8`](Self::from_u8).
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::NotUsed => 0x00,
            Self::Dialup => 0x01,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (Table 37).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NotUsed => "not used",
            Self::Dialup => "dialup",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(IspAccessMode, Reserved);

/// ISP_access_mode_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct IspAccessModeDescriptor<'a> {
    /// `access_mode` (Table 37).
    pub access_mode: IspAccessMode,
    /// Bytes after `access_mode` inside `descriptor_length`; the spec defines
    /// none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for IspAccessModeDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "IspAccessModeDescriptor",
            "unexpected tag for ISP_access_mode_descriptor",
        )?;
        let (mode, trailing) = body.split_first().ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "ISP_access_mode_descriptor body is empty",
        })?;
        Ok(Self {
            access_mode: IspAccessMode::from_u8(*mode),
            trailing,
        })
    }
}

impl Serialize for IspAccessModeDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN + self.trailing.len()
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
        buf[2] = self.access_mode.to_u8();
        buf[3..len].copy_from_slice(self.trailing);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for IspAccessModeDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ISP_ACCESS_MODE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: access_mode "dialup" (0x01) and 0xBC.
        for (bytes, mode) in [
            ([TAG, 1, 0x01], IspAccessMode::Dialup),
            ([TAG, 1, 0xBC], IspAccessMode::Reserved(0xBC)),
            ([TAG, 1, 0x00], IspAccessMode::NotUsed),
        ] {
            let d = IspAccessModeDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.access_mode, mode);
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn trailing_round_trip_and_rejects_empty() {
        let bytes = [TAG, 3, 0x01, 0xAA, 0xBB];
        let d = IspAccessModeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.trailing, &[0xAA, 0xBB]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert!(IspAccessModeDescriptor::parse(&[TAG, 0]).is_err());
    }

    #[test]
    fn labels_and_lossless_enum() {
        assert_eq!(IspAccessMode::Dialup.to_string(), "dialup");
        assert_eq!(IspAccessMode::Reserved(0xBC).to_string(), "reserved(0xBC)");
        for b in 0..=u8::MAX {
            assert_eq!(IspAccessMode::from_u8(b).to_u8(), b);
        }
    }
}
