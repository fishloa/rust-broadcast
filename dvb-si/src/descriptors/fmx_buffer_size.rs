//! FmxBufferSize Descriptor — ISO/IEC 13818-1 §2.6.50, Table 2-80 (tag 0x22).
//!
//! Carries a `DefaultFlexMuxBufferDescriptor()` followed by a loop of
//! `FlexMuxBufferDescriptor()` entries, both defined in ISO/IEC 14496-1
//! §11.2. That section is **not vendored** in this repo (`private/specs/`
//! has no 14496-1), so neither structure's field layout is transcribed here
//! and this module deliberately does not invent one: it splits the body
//! structurally (see [`FmxBufferSizeDescriptor`]) and exposes both parts as
//! borrowed byte ranges.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for FmxBufferSize_descriptor.
pub const TAG: u8 = 0x22;
const HEADER_LEN: usize = 2;

/// Byte width of `DefaultFlexMuxBufferDescriptor()` (ISO/IEC 14496-1 §11.2):
/// a single 24-bit `FB_DefaultBufferSize`.
///
/// Fixed-size by construction: ISO/IEC 13818-1 Table 2-80 carries no length
/// field for it, so its width must be a constant for the body to be
/// splittable at all. The clause is not vendored in `private/specs/`, so the
/// transcription this constant comes from is
/// `docs/descriptors/iso_14496_1/11_2-flexmux-buffer-descriptors.md` — which
/// records that it is corroborated by three independent implementations
/// rather than copied from the standard text.
const DEFAULT_FLEX_MUX_BUFFER_DESCRIPTOR_LEN: usize = 3;

/// Byte width of one `FlexMuxBufferDescriptor()` (ISO/IEC 14496-1 §11.2) —
/// the `i += 4` step in Table 2-80's loop. Same transcription as above.
const FLEX_MUX_BUFFER_DESCRIPTOR_LEN: usize = 4;

/// FmxBufferSize Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct FmxBufferSizeDescriptor<'a> {
    /// `DefaultFlexMuxBufferDescriptor()` bytes.
    ///
    /// Exactly 3 bytes (`DEFAULT_FLEX_MUX_BUFFER_DESCRIPTOR_LEN`): ISO/IEC
    /// 13818-1 Table 2-80 gives no length field for this structure, but
    /// ISO/IEC 14496-1 §11.2 defines it as a fixed-size record, so it is
    /// always present — a body too short to hold it is rejected rather than
    /// reported as "default absent" (r03-W15).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub default_flex_mux_buffer_descriptor: &'a [u8],
    /// The `FlexMuxBufferDescriptor()` loop, always a whole number of
    /// 4-byte entries (`FLEX_MUX_BUFFER_DESCRIPTOR_LEN`).
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
        // Table 2-80 has no length field at all: the whole body is
        // `DefaultFlexMuxBufferDescriptor()` followed by `i += 4` entries.
        // The split is therefore structural, not a heuristic: the default is
        // the fixed-size record at the front and everything after it is the
        // entry loop. A body that cannot even hold the default, or whose
        // remainder is not a whole number of entries, is not a valid instance
        // of this descriptor (r03-W15).
        let (default_flex_mux_buffer_descriptor, flex_mux_buffer_descriptors) = body
            .split_at_checked(DEFAULT_FLEX_MUX_BUFFER_DESCRIPTOR_LEN)
            .ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "FmxBufferSize_descriptor too short for DefaultFlexMuxBufferDescriptor",
            })?;
        if flex_mux_buffer_descriptors.len() % FLEX_MUX_BUFFER_DESCRIPTOR_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "FmxBufferSize_descriptor body is not a whole number of FlexMuxBufferDescriptor entries",
            });
        }
        Ok(Self {
            default_flex_mux_buffer_descriptor,
            flex_mux_buffer_descriptors,
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
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        debug_assert_eq!(
            self.default_flex_mux_buffer_descriptor.len(),
            DEFAULT_FLEX_MUX_BUFFER_DESCRIPTOR_LEN
        );
        debug_assert_eq!(
            self.flex_mux_buffer_descriptors.len() % FLEX_MUX_BUFFER_DESCRIPTOR_LEN,
            0
        );
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
    fn parse_default_only() {
        // 3-byte default, no entries: descriptor_length == 3.
        let bytes = [TAG, 3, 0xAA, 0xBB, 0xCC];
        let d = FmxBufferSizeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.default_flex_mux_buffer_descriptor, &[0xAA, 0xBB, 0xCC]);
        assert!(d.flex_mux_buffer_descriptors.is_empty());
    }

    #[test]
    fn parse_with_one_entry() {
        // 3-byte default + one 4-byte entry: descriptor_length == 7.
        let bytes = [TAG, 7, 0xAA, 0xBB, 0xCC, 0x01, 0x02, 0x03, 0x04];
        let d = FmxBufferSizeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.default_flex_mux_buffer_descriptor, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(d.flex_mux_buffer_descriptors, &[0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn parse_rejects_body_shorter_than_default() {
        // The default record is mandatory (no optional marker in Table 2-80),
        // so an empty or 1-2 byte body is not a valid descriptor. The old
        // `len % 4` heuristic reported these as "default absent" (r03-W15).
        for len in [0usize, 1, 2] {
            let mut bytes = vec![TAG, len as u8];
            bytes.extend(std::iter::repeat_n(0u8, len));
            assert!(matches!(
                FmxBufferSizeDescriptor::parse(&bytes).unwrap_err(),
                Error::InvalidDescriptor { tag: TAG, .. }
            ));
        }
    }

    #[test]
    fn parse_rejects_partial_entry() {
        // 3 + 3 is not a whole number of 4-byte entries.
        let bytes = [TAG, 6, 0xAA, 0xBB, 0xCC, 0x01, 0x02, 0x03];
        assert!(matches!(
            FmxBufferSizeDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = FmxBufferSizeDescriptor::parse(&[0x02, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = FmxBufferSizeDescriptor {
            default_flex_mux_buffer_descriptor: &[0xDD, 0xEE, 0xFF],
            flex_mux_buffer_descriptors: &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[1], 3 + 8);
        let reparsed = FmxBufferSizeDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = FmxBufferSizeDescriptor {
            default_flex_mux_buffer_descriptor: &[0, 0, 0],
            flex_mux_buffer_descriptors: &[1, 2, 3, 4],
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
