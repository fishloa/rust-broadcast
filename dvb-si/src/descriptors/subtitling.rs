//! Subtitling Descriptor — ETSI EN 300 468 §6.2.42 (tag 0x59).
//!
//! Carried inside PMT's ES_info loop. Enumerates DVB subtitle services:
//! one entry per 3-char language code + subtitling_type + composition/
//! ancillary page triple (8 bytes).

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::LangCode;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for subtitling_descriptor.
pub const TAG: u8 = 0x59;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 8;

/// One subtitling component.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubtitlingEntry {
    /// ISO 639-2 language code.
    pub language_code: LangCode,
    /// subtitling_type byte (ETSI EN 300 468 §6.2.42): 0x01 = EBU teletext subtitles,
    /// 0x10..=0x13 = DVB subtitles, etc.
    pub subtitling_type: u8,
    /// composition_page_id.
    pub composition_page_id: u16,
    /// ancillary_page_id.
    pub ancillary_page_id: u16,
}

/// Subtitling Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubtitlingDescriptor {
    /// Entries in wire order.
    pub entries: Vec<SubtitlingEntry>,
}

impl<'a> Parse<'a> for SubtitlingDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SubtitlingDescriptor",
            "unexpected tag for subtitling_descriptor",
        )?;
        if body.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "subtitling_descriptor length must be a multiple of 8",
            });
        }
        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            entries.push(SubtitlingEntry {
                language_code: LangCode([chunk[0], chunk[1], chunk[2]]),
                subtitling_type: chunk[3],
                composition_page_id: u16::from_be_bytes([chunk[4], chunk[5]]),
                ancillary_page_id: u16::from_be_bytes([chunk[6], chunk[7]]),
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for SubtitlingDescriptor {
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
        buf[1] = (self.entries.len() * ENTRY_LEN) as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            buf[pos..pos + 3].copy_from_slice(&e.language_code.0);
            buf[pos + 3] = e.subtitling_type;
            buf[pos + 4..pos + 6].copy_from_slice(&e.composition_page_id.to_be_bytes());
            buf[pos + 6..pos + 8].copy_from_slice(&e.ancillary_page_id.to_be_bytes());
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for SubtitlingDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SUBTITLING";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        let bytes = [TAG, 8, b'e', b'n', b'g', 0x10, 0x00, 0x01, 0x00, 0x02];
        let d = SubtitlingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].language_code, LangCode(*b"eng"));
        assert_eq!(d.entries[0].subtitling_type, 0x10);
        assert_eq!(d.entries[0].composition_page_id, 1);
        assert_eq!(d.entries[0].ancillary_page_id, 2);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            SubtitlingDescriptor::parse(&[0x5A, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x5A, .. }
        ));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_8() {
        let bytes = [TAG, 7, 0, 0, 0, 0, 0, 0, 0];
        assert!(matches!(
            SubtitlingDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = SubtitlingDescriptor {
            entries: vec![
                SubtitlingEntry {
                    language_code: LangCode(*b"fra"),
                    subtitling_type: 0x10,
                    composition_page_id: 0x1234,
                    ancillary_page_id: 0x5678,
                },
                SubtitlingEntry {
                    language_code: LangCode(*b"deu"),
                    subtitling_type: 0x20,
                    composition_page_id: 0,
                    ancillary_page_id: 0,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(SubtitlingDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn empty_descriptor_valid() {
        let d = SubtitlingDescriptor::parse(&[TAG, 0]).unwrap();
        assert_eq!(d.entries.len(), 0);
    }
}
