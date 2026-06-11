//! Content Descriptor — ETSI EN 300 468 §6.2.9 (tag 0x54).
//!
//! Carried inside EIT. Classifies the event's genre via a two-nibble
//! content type plus an 8-bit broadcaster-specific user byte.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for content_descriptor.
pub const TAG: u8 = 0x54;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 2;

/// One content classification entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ContentEntry {
    /// content_nibble_level_1 (4 bits) — broad genre (ETSI Table 29).
    pub nibble_1: u8,
    /// content_nibble_level_2 (4 bits) — sub-genre.
    pub nibble_2: u8,
    /// Broadcaster-specific user byte.
    pub user_byte: u8,
}

/// Content Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ContentDescriptor {
    /// Entries in wire order. EIT events can carry multiple genre entries.
    pub entries: Vec<ContentEntry>,
}

impl<'a> Parse<'a> for ContentDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ContentDescriptor",
            "unexpected tag for ContentDescriptor",
        )?;

        if body.len() % 2 != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 2",
            });
        }

        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);

        for chunk in body.chunks_exact(ENTRY_LEN) {
            entries.push(ContentEntry {
                nibble_1: chunk[0] >> 4,
                nibble_2: chunk[0] & 0x0F,
                user_byte: chunk[1],
            });
        }

        Ok(Self { entries })
    }
}

impl Serialize for ContentDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.entries.len() * ENTRY_LEN
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
        let mut pos = HEADER_LEN;
        for entry in &self.entries {
            buf[pos] = (entry.nibble_1 << 4) | entry.nibble_2;
            buf[pos + 1] = entry.user_byte;
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for ContentDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CONTENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry_extracts_nibbles_and_user_byte() {
        let result = ContentDescriptor::parse(&[TAG, 2, 0x31, 0xFF]).unwrap();
        assert_eq!(result.entries.len(), 1);
        let e = result.entries[0];
        assert_eq!(e.nibble_1, 3);
        assert_eq!(e.nibble_2, 1);
        assert_eq!(e.user_byte, 0xFF);
    }

    #[test]
    fn parse_multiple_entries_preserves_order() {
        let bytes = [TAG, 6, 0x31, 0xAA, 0x42, 0xBB, 0x53, 0xCC];
        let result = ContentDescriptor::parse(&bytes).unwrap();
        assert_eq!(result.entries.len(), 3);
        assert_eq!(result.entries[0].nibble_1, 3);
        assert_eq!(result.entries[0].nibble_2, 1);
        assert_eq!(result.entries[0].user_byte, 0xAA);
        assert_eq!(result.entries[1].nibble_1, 4);
        assert_eq!(result.entries[1].nibble_2, 2);
        assert_eq!(result.entries[1].user_byte, 0xBB);
        assert_eq!(result.entries[2].nibble_1, 5);
        assert_eq!(result.entries[2].nibble_2, 3);
        assert_eq!(result.entries[2].user_byte, 0xCC);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            ContentDescriptor::parse(&[0x55, 2, 0x00, 0x00]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x55, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_header() {
        assert!(matches!(
            ContentDescriptor::parse(&[TAG]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_body_truncation() {
        // length=4 means body is 4 bytes, but only 1 byte follows header
        assert!(matches!(
            ContentDescriptor::parse(&[TAG, 4, 0x01]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_2() {
        // length=3 is odd, can't form complete entries
        assert!(matches!(
            ContentDescriptor::parse(&[TAG, 3, 0x01, 0x02, 0x03]).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let original = ContentDescriptor {
            entries: vec![
                ContentEntry {
                    nibble_1: 3,
                    nibble_2: 1,
                    user_byte: 0xAA,
                },
                ContentEntry {
                    nibble_1: 4,
                    nibble_2: 2,
                    user_byte: 0xBB,
                },
            ],
        };
        let mut buf = vec![0u8; original.serialized_len()];
        original.serialize_into(&mut buf).unwrap();
        let parsed = ContentDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn empty_descriptor_valid() {
        let bytes = [TAG, 0];
        let result = ContentDescriptor::parse(&bytes).unwrap();
        assert!(result.entries.is_empty());
    }
}
