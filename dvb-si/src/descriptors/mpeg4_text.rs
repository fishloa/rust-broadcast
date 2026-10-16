//! MPEG-4 text descriptor — Rec. ITU-T H.222.0 (08/2023) | ISO/IEC 13818-1:2023
//! §2.6.70–2.6.71, Table 2-96 (tag 0x2D); syntax in
//! `docs/descriptors/iso_13818_1/2d-2e-mpeg-4-text-and-audio-extension-descriptors.md`.
//!
//! The whole body is `textConfig()`, the `TextConfig()` of ISO/IEC 14496-17
//! (not vendored): it is carried as one opaque byte run, by design.

use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag of `MPEG-4_text_descriptor` (Table 2-45: 45).
pub const TAG: u8 = 0x2D;
const HEADER_LEN: usize = 2;

/// `MPEG-4_text_descriptor()` — Table 2-96.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Mpeg4TextDescriptor<'a> {
    /// `textConfig()` — the ISO/IEC 14496-17 `TextConfig()` of the associated
    /// text stream (opaque: that standard is not vendored).
    pub text_config: &'a [u8],
}

impl<'a> Mpeg4TextDescriptor<'a> {
    /// Wrap a `TextConfig()` byte run.
    #[must_use]
    pub const fn new(text_config: &'a [u8]) -> Self {
        Self { text_config }
    }
}

impl<'a> Parse<'a> for Mpeg4TextDescriptor<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Mpeg4TextDescriptor",
            "unexpected tag for MPEG-4_text_descriptor",
        )?;
        Ok(Self { text_config: body })
    }
}

impl Serialize for Mpeg4TextDescriptor<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.text_config.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, self.text_config.len())?;
        buf[HEADER_LEN..len].copy_from_slice(self.text_config);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for Mpeg4TextDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MPEG4_TEXT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bounds() {
        let b = [TAG, 3, 0xAA, 0xBB, 0xCC];
        let d = Mpeg4TextDescriptor::parse(&b).unwrap();
        assert_eq!(d.text_config, [0xAA, 0xBB, 0xCC]);
        let mut out = vec![0u8; d.serialized_len()];
        assert_eq!(d.serialize_into(&mut out).unwrap(), 5);
        assert_eq!(out, b);
        assert!(Mpeg4TextDescriptor::parse(&[0x2C, 0]).is_err());
        assert!(Mpeg4TextDescriptor::parse(&[TAG, 4, 1]).is_err());
        let big = vec![0u8; 256];
        let d = Mpeg4TextDescriptor::new(&big);
        let mut out = vec![0u8; 300];
        assert!(d.serialize_into(&mut out).is_err());
        assert!(d.serialize_into(&mut out[..10]).is_err());
    }
}
