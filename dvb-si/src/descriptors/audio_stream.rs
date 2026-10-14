//! Audio Stream Descriptor — ISO/IEC 13818-1 §2.6.4 (tag 0x03).
//!
//! Describes the elementary stream as an MPEG audio stream.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for audio_stream_descriptor.
pub const TAG: u8 = 0x03;
const HEADER_LEN: usize = 2;
/// Default of the 3-bit `reserved` tail of the body byte: H.222.0 clause
/// 2.1 "reserved" = `1` (all 9 editions 07/1995-08/2023; audit §5 — layout
/// unchanged in every edition). Parse keeps whatever a sender wrote.
pub const RESERVED_DEFAULT: u8 = 0b111;
const TAIL_RESERVED_MASK: u8 = 0x07;
const BODY_LEN: u8 = 1;

/// Audio Stream Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct AudioStreamDescriptor {
    /// Free format flag.
    pub free_format_flag: bool,
    /// MPEG audio ID.
    pub id: bool,
    /// Layer (0=reserved).
    pub layer: u8,
    /// Variable rate audio indicator.
    pub variable_rate_audio_indicator: bool,
    /// `reserved(3)` tail (3 bits, right-aligned). Captured on parse,
    /// re-emitted verbatim; [`AudioStreamDescriptor::new`] uses
    /// [`RESERVED_DEFAULT`].
    pub reserved: u8,
}

impl AudioStreamDescriptor {
    /// Build with the spec-default reserved bits.
    #[must_use]
    pub const fn new(
        free_format_flag: bool,
        id: bool,
        layer: u8,
        variable_rate_audio_indicator: bool,
    ) -> Self {
        Self {
            free_format_flag,
            id,
            layer,
            variable_rate_audio_indicator,
            reserved: RESERVED_DEFAULT,
        }
    }
}

impl<'a> Parse<'a> for AudioStreamDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AudioStreamDescriptor",
            "unexpected tag for audio_stream_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "audio_stream_descriptor length must equal 1",
            });
        }
        let b = body[0];
        Ok(Self {
            free_format_flag: (b & 0x80) != 0,
            id: (b & 0x40) != 0,
            layer: (b >> 4) & 0x03,
            variable_rate_audio_indicator: (b & 0x08) != 0,
            reserved: b & TAIL_RESERVED_MASK,
        })
    }
}

impl Serialize for AudioStreamDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
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
        buf[1] = BODY_LEN;
        buf[HEADER_LEN] = ((self.free_format_flag as u8) << 7)
            | ((self.id as u8) << 6)
            | (broadcast_common::len::fit_mask(self.layer, 0x03, "layer")? << 4)
            | ((self.variable_rate_audio_indicator as u8) << 3)
            | broadcast_common::len::fit_mask(self.reserved, TAIL_RESERVED_MASK, "reserved")?;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for AudioStreamDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AUDIO_STREAM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            1,
            0b1110_1000, // free=1, id=1, layer=2, variable=1, reserved=0
        ];
        let d = AudioStreamDescriptor::parse(&bytes).unwrap();
        assert!(d.free_format_flag);
        assert!(d.id);
        assert_eq!(d.layer, 2);
        assert!(d.variable_rate_audio_indicator);
    }

    #[test]
    fn serialize_round_trip() {
        let d = AudioStreamDescriptor::new(false, true, 3, false);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = AudioStreamDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = AudioStreamDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = AudioStreamDescriptor::parse(&[0x04, 1, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x04, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = AudioStreamDescriptor::new(false, false, 0, false);
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn audio_stream_reserved_preserved() {
        for r in 0u8..=7 {
            let bytes = [TAG, 1, 0b1110_1000 | r];
            let d = AudioStreamDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved, r);
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes);
        }
    }

    /// Default-constructed values carry the H.222.0 `reserved` = 1 fill.
    #[test]
    fn audio_stream_default_is_spec_value() {
        let d = AudioStreamDescriptor::new(true, true, 2, true);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 1, 0b1110_1111]);
    }
}
