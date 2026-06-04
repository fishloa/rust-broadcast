//! Component Descriptor — ETSI EN 300 468 §6.2.8 (tag 0x50).
//!
//! Describes one elementary component (audio/video/data/teletext/subtitle)
//! with a language code and free-text label. Carried inside EIT event
//! descriptor loops.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for component_descriptor.
pub const TAG: u8 = 0x50;
const HEADER_LEN: usize = 2;
/// stream_content(1) + component_type(1) + component_tag(1) + language_code(3) = 6
const PRE_TEXT_LEN: usize = 6;
const STREAM_CONTENT_MASK: u8 = 0x0F;

/// Component Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ComponentDescriptor<'a> {
    /// 4-bit stream_content_ext (high nibble) — combines with `stream_content`
    /// to identify the component (EN 300 468 §6.2.8 Table 25).
    pub stream_content_ext: u8,
    /// 4-bit stream_content (0x01 video, 0x02 audio, 0x03 teletext, …).
    pub stream_content: u8,
    /// component_type byte (sub-classification inside the stream_content).
    pub component_type: u8,
    /// component_tag for cross-reference with stream_identifier_descriptor.
    pub component_tag: u8,
    /// ISO 639-2 language code.
    pub language_code: [u8; 3],
    /// Raw DVB-encoded text label for this component.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub text: &'a [u8],
}

impl<'a> Parse<'a> for ComponentDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "ComponentDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for component_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "ComponentDescriptor body",
            });
        }
        if length < PRE_TEXT_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "component_descriptor body shorter than minimum 6 bytes",
            });
        }
        let stream_content_ext = bytes[HEADER_LEN] >> 4;
        let stream_content = bytes[HEADER_LEN] & STREAM_CONTENT_MASK;
        let component_type = bytes[HEADER_LEN + 1];
        let component_tag = bytes[HEADER_LEN + 2];
        let language_code = [
            bytes[HEADER_LEN + 3],
            bytes[HEADER_LEN + 4],
            bytes[HEADER_LEN + 5],
        ];
        let text = &bytes[HEADER_LEN + PRE_TEXT_LEN..end];
        Ok(Self {
            stream_content_ext,
            stream_content,
            component_type,
            component_tag,
            language_code,
            text,
        })
    }
}

impl Serialize for ComponentDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + PRE_TEXT_LEN + self.text.len()
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
        // High nibble = stream_content_ext, low nibble = stream_content (§6.2.8).
        buf[HEADER_LEN] =
            (self.stream_content_ext << 4) | (self.stream_content & STREAM_CONTENT_MASK);
        buf[HEADER_LEN + 1] = self.component_type;
        buf[HEADER_LEN + 2] = self.component_tag;
        buf[HEADER_LEN + 3..HEADER_LEN + 6].copy_from_slice(&self.language_code);
        buf[HEADER_LEN + PRE_TEXT_LEN..len].copy_from_slice(self.text);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for ComponentDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_all_fields() {
        let bytes = [
            TAG, 12, 0x02, 0x01, 0x42, b'e', b'n', b'g', b'S', b'T', b'E', b'R', b'E', b'O',
        ];
        let d = ComponentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.stream_content, 2);
        assert_eq!(d.component_type, 0x01);
        assert_eq!(d.component_tag, 0x42);
        assert_eq!(&d.language_code, b"eng");
        assert_eq!(d.text, b"STEREO");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            ComponentDescriptor::parse(&[0x51, 6, 0, 0, 0, b'e', b'n', b'g']).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x51, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_body() {
        let bytes = [TAG, 5, 0x01, 0x00, 0x00, b'e', b'n'];
        assert!(matches!(
            ComponentDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn parse_rejects_truncated_buffer() {
        let bytes = [TAG, 6, 0x01];
        assert!(matches!(
            ComponentDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_with_empty_text_valid() {
        let bytes = [TAG, 6, 0x01, 0x01, 0x01, b'e', b'n', b'g'];
        let d = ComponentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.text, &[] as &[u8]);
    }

    #[test]
    fn serialize_round_trip() {
        let d = ComponentDescriptor {
            stream_content_ext: 0x0F,
            stream_content: 0x03,
            component_type: 0x10,
            component_tag: 0x05,
            language_code: *b"fra",
            text: b"Sous-titres",
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(ComponentDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn stream_content_masked_to_low_nibble() {
        // Byte = 0xF2 → high nibble is stream_content_ext (0xF), low nibble is stream_content=2.
        let bytes = [TAG, 6, 0xF2, 0x00, 0x00, b'e', b'n', b'g'];
        let d = ComponentDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.stream_content, 2);
    }
}
