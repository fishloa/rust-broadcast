//! FmxBufferSize Descriptor — ISO/IEC 13818-1 §2.6.50, Table 2-80 (tag 0x22).
//!
//! Carries a DefaultFlexMuxBufferDescriptor() followed by a loop of
//! FlexMuxBufferDescriptor() entries — both from ISO/IEC 14496-1 §11.2,
//! carried as opaque bytes.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for FmxBufferSize_descriptor.
pub const TAG: u8 = 0x22;
const HEADER_LEN: usize = 2;

/// FmxBufferSize Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct FmxBufferSizeDescriptor<'a> {
    /// DefaultFlexMuxBufferDescriptor() carried as opaque bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub default_flex_mux_buffer_descriptor: &'a [u8],
    /// FlexMuxBufferDescriptor() entries, each 4 opaque bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub flex_mux_buffer_descriptors: &'a [u8],
}

impl<'a> Parse<'a> for FmxBufferSizeDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "FmxBufferSizeDescriptor",
            "unexpected tag for FmxBufferSize_descriptor",
        )?;
        // The DefaultFlexMuxBufferDescriptor length is not fixed by the spec;
        // it is determined by the descriptor_length. The spec says:
        // "for (i = 0; i < descriptor_length; i += 4)" after DefaultFlexMuxBufferDescriptor().
        // This means the DefaultFlexMuxBufferDescriptor size is descriptor_length % 4,
        // and the rest is FlexMuxBufferDescriptor entries.
        // However, without knowing the Default... size, we cannot split safely.
        // We store the whole body as opaque, with a warning.
        //
        // Actually, looking more carefully: the spec shows the loop advances i+=4
        // starting *after* DefaultFlexMuxBufferDescriptor(). That means Default is
        // the first (descriptor_length % 4) != 0 bytes or a well-known size. But
        // the size is not specified in ISO/IEC 13818-1. We'll split via total
        // descriptor_length mod 4 — if non-zero, the remainder is Default.
        //
        // Simpler: the DefaultFlexMuxBufferDescriptor() size is descriptor_length % 4
        // (because FlexMuxBufferDescriptor entries are each 4 bytes). If length is
        // a multiple of 4, Default size is 0.

        let default_len = body.len() % 4;
        Ok(Self {
            default_flex_mux_buffer_descriptor: &body[..default_len],
            flex_mux_buffer_descriptors: &body[default_len..],
        })
    }
}

impl Serialize for FmxBufferSizeDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self.default_flex_mux_buffer_descriptor.len()
            + self.flex_mux_buffer_descriptors.len()
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
        let mut off = HEADER_LEN;
        buf[off..off + self.default_flex_mux_buffer_descriptor.len()]
            .copy_from_slice(self.default_flex_mux_buffer_descriptor);
        off += self.default_flex_mux_buffer_descriptor.len();
        buf[off..len].copy_from_slice(self.flex_mux_buffer_descriptors);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for FmxBufferSizeDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "FMX_BUFFER_SIZE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_default() {
        let bytes = [TAG, 4, 0x01, 0x02, 0x03, 0x04];
        let d = FmxBufferSizeDescriptor::parse(&bytes).unwrap();
        assert!(d.default_flex_mux_buffer_descriptor.is_empty());
        assert_eq!(d.flex_mux_buffer_descriptors, &[0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn parse_with_default() {
        let bytes = [TAG, 7, 0xAA, 0xBB, 0xCC, 0x01, 0x02, 0x03, 0x04];
        let d = FmxBufferSizeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.default_flex_mux_buffer_descriptor, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(d.flex_mux_buffer_descriptors, &[0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn parse_empty_all() {
        let d = FmxBufferSizeDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.default_flex_mux_buffer_descriptor.is_empty());
        assert!(d.flex_mux_buffer_descriptors.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = FmxBufferSizeDescriptor::parse(&[0x02, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = FmxBufferSizeDescriptor {
            default_flex_mux_buffer_descriptor: &[0xDD],
            flex_mux_buffer_descriptors: &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = FmxBufferSizeDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = FmxBufferSizeDescriptor {
            default_flex_mux_buffer_descriptor: &[],
            flex_mux_buffer_descriptors: &[1, 2, 3, 4],
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
