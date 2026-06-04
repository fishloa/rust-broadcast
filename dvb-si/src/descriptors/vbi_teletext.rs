//! VBI Teletext Descriptor — ETSI EN 300 468 §6.2.48 (tag 0x46).
//!
//! Table 108 (PDF p. 111). Identical wire layout to the teletext_descriptor
//! (Table 101): a loop of 5-byte entries, each a 3-char ISO 639 language code
//! plus teletext_type (5 bits) / magazine_number (3 bits) / page_number
//! (8 bits). Signals teletext also carried in the analogue VBI lines.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for VBI_teletext_descriptor.
pub const TAG: u8 = 0x46;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 5;
const LANG_LEN: usize = 3;
/// Maximum body length expressible in the 8-bit `descriptor_length` field.
const MAX_BODY_LEN: usize = u8::MAX as usize;

/// One VBI teletext component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VbiTeletextEntry {
    /// ISO 639-2 language code of this teletext service.
    pub language_code: [u8; 3],
    /// 5-bit teletext_type (EN 300 468 Table 102).
    pub teletext_type: u8,
    /// 3-bit teletext_magazine_number.
    pub magazine_number: u8,
    /// 8-bit BCD teletext_page_number.
    pub page_number: u8,
}

/// VBI Teletext Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VbiTeletextDescriptor {
    /// Teletext components in wire order.
    pub entries: Vec<VbiTeletextEntry>,
}

impl<'a> Parse<'a> for VbiTeletextDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "VbiTeletextDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for VBI_teletext_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 5",
            });
        }
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "VbiTeletextDescriptor body",
            });
        }
        let body = &bytes[HEADER_LEN..end];
        let mut entries = Vec::with_capacity(length / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            let language_code = [chunk[0], chunk[1], chunk[2]];
            let type_and_mag = chunk[LANG_LEN];
            entries.push(VbiTeletextEntry {
                language_code,
                teletext_type: (type_and_mag >> 3) & 0x1F,
                magazine_number: type_and_mag & 0x07,
                page_number: chunk[LANG_LEN + 1],
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for VbiTeletextDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ENTRY_LEN * self.entries.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let body_len = ENTRY_LEN * self.entries.len();
        // 8-bit descriptor_length field: error rather than silently truncate.
        if body_len > MAX_BODY_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: MAX_BODY_LEN,
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            buf[pos..pos + LANG_LEN].copy_from_slice(&e.language_code);
            buf[pos + LANG_LEN] = ((e.teletext_type & 0x1F) << 3) | (e.magazine_number & 0x07);
            buf[pos + LANG_LEN + 1] = e.page_number;
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for VbiTeletextDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (ENTRY_LEN * self.entries.len()) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        let bytes = [TAG, 5, b'e', b'n', b'g', (1 << 3) | 2, 0x10];
        let d = VbiTeletextDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(&d.entries[0].language_code, b"eng");
        assert_eq!(d.entries[0].teletext_type, 1);
        assert_eq!(d.entries[0].magazine_number, 2);
        assert_eq!(d.entries[0].page_number, 0x10);
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = [
            TAG,
            10,
            b'e',
            b'n',
            b'g',
            (1 << 3) | 1,
            0x10,
            b'f',
            b'r',
            b'a',
            (2 << 3) | 1,
            0x20,
        ];
        let d = VbiTeletextDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].teletext_type, 2);
        assert_eq!(&d.entries[1].language_code, b"fra");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            VbiTeletextDescriptor::parse(&[0x47, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x47, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let bytes = [TAG, 5, b'e', b'n'];
        assert!(matches!(
            VbiTeletextDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_5() {
        let bytes = [TAG, 4, 0, 0, 0, 0];
        assert!(matches!(
            VbiTeletextDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn empty_descriptor_valid() {
        let d = VbiTeletextDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = VbiTeletextDescriptor {
            entries: vec![VbiTeletextEntry {
                language_code: *b"fra",
                teletext_type: 2,
                magazine_number: 8 & 0x07,
                page_number: 0x88,
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(VbiTeletextDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        // 52 entries = 260 body bytes, past the u8 length field.
        let d = VbiTeletextDescriptor {
            entries: vec![
                VbiTeletextEntry {
                    language_code: *b"eng",
                    teletext_type: 1,
                    magazine_number: 1,
                    page_number: 0,
                };
                52
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = VbiTeletextDescriptor {
            entries: vec![VbiTeletextEntry {
                language_code: *b"eng",
                teletext_type: 2,
                magazine_number: 1,
                page_number: 0x10,
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: VbiTeletextDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }
}
