//! VBI Teletext Descriptor — ETSI EN 300 468 §6.2.48 (tag 0x46).
//!
//! Table 108 (PDF p. 111). Identical wire layout to the teletext_descriptor
//! (Table 101): a loop of 5-byte entries, each a 3-char ISO 639 language code
//! plus teletext_type (5 bits) / magazine_number (3 bits) / page_number
//! (8 bits). Signals teletext also carried in the analogue VBI lines.

use super::descriptor_body;
use super::teletext::{ENTRY_LEN, HEADER_LEN, TeletextEntry, parse_entries, serialize_entries};
use crate::error::Result;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for VBI_teletext_descriptor.
pub const TAG: u8 = 0x46;

/// One VBI teletext component — the same entry as the teletext descriptor's
/// (EN 300 468 Table 108 vs Table 101), so one type serves both.
pub type VbiTeletextEntry = TeletextEntry;

/// VBI Teletext Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VbiTeletextDescriptor {
    /// Teletext components in wire order.
    pub entries: Vec<VbiTeletextEntry>,
}

impl<'a> Parse<'a> for VbiTeletextDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "VbiTeletextDescriptor",
            "unexpected tag for VBI_teletext_descriptor",
        )?;
        let entries = parse_entries(body, TAG, "descriptor_length must be a multiple of 5")?;
        Ok(Self { entries })
    }
}

impl Serialize for VbiTeletextDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ENTRY_LEN * self.entries.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_entries(&self.entries, buf, TAG)
    }
}
impl crate::traits::DescriptorDef<'_> for VbiTeletextDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "VBI_TELETEXT";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::teletext::TeletextType;
    use crate::error::Error;
    use crate::text::LangCode;

    #[test]
    fn parse_single_entry() {
        let bytes = [TAG, 5, b'e', b'n', b'g', (1 << 3) | 2, 0x10];
        let d = VbiTeletextDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].language_code, LangCode(*b"eng"));
        assert_eq!(d.entries[0].teletext_type, TeletextType::InitialPage);
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
        assert_eq!(d.entries[1].teletext_type, TeletextType::SubtitlePage);
        assert_eq!(d.entries[1].language_code, LangCode(*b"fra"));
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
                language_code: LangCode(*b"fra"),
                teletext_type: TeletextType::SubtitlePage,
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
                    language_code: LangCode(*b"eng"),
                    teletext_type: TeletextType::Reserved(1),
                    magazine_number: 1,
                    page_number: 0,
                };
                52
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = VbiTeletextDescriptor {
            entries: vec![VbiTeletextEntry {
                language_code: LangCode(*b"eng"),
                teletext_type: TeletextType::SubtitlePage,
                magazine_number: 1,
                page_number: 0x10,
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
