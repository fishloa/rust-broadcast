//! Teletext Descriptor — ETSI EN 300 468 §6.2.44 (tag 0x56).
//!
//! Carried inside PMT's ES_info loop. Enumerates teletext components: one
//! entry per 3-char language code + type/magazine/page triple (5 bytes).

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::LangCode;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for teletext_descriptor.
pub const TAG: u8 = 0x56;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 5;
const LANG_LEN: usize = 3;

/// Teletext type — ETSI EN 300 468 Table 102.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TeletextType {
    /// 0x01 — initial teletext page.
    InitialPage,
    /// 0x02 — teletext subtitle page.
    SubtitlePage,
    /// 0x03 — additional information page.
    AdditionalInformationPage,
    /// 0x04 — programme schedule page.
    ProgrammeSchedulePage,
    /// 0x05 — teletext subtitle page for hearing impaired people.
    HearingImpairedSubtitlePage,
    /// Reserved/unallocated wire value, preserved verbatim for round-trip.
    Reserved(u8),
}

impl TeletextType {
    #[must_use]
    /// Creates a value from a wire byte, preserving every possible
    /// byte value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::InitialPage,
            0x02 => Self::SubtitlePage,
            0x03 => Self::AdditionalInformationPage,
            0x04 => Self::ProgrammeSchedulePage,
            0x05 => Self::HearingImpairedSubtitlePage,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the wire byte for this value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::InitialPage => 0x01,
            Self::SubtitlePage => 0x02,
            Self::AdditionalInformationPage => 0x03,
            Self::ProgrammeSchedulePage => 0x04,
            Self::HearingImpairedSubtitlePage => 0x05,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns a human-readable spec name for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::InitialPage => "initial teletext page",
            Self::SubtitlePage => "teletext subtitle page",
            Self::AdditionalInformationPage => "additional information page",
            Self::ProgrammeSchedulePage => "programme schedule page",
            Self::HearingImpairedSubtitlePage => {
                "teletext subtitle page for hearing impaired people"
            }
            Self::Reserved(_) => "reserved",
        }
    }
}

/// One teletext component.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TeletextEntry {
    /// ISO 639-2 language code of this teletext service.
    pub language_code: LangCode,
    /// 5-bit teletext_type (ETSI Table 102).
    pub teletext_type: TeletextType,
    /// 3-bit teletext_magazine_number.
    pub magazine_number: u8,
    /// 8-bit BCD teletext_page_number.
    pub page_number: u8,
}

/// Teletext Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TeletextDescriptor {
    /// Teletext components listed in wire order.
    pub entries: Vec<TeletextEntry>,
}

impl<'a> Parse<'a> for TeletextDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "TeletextDescriptor",
            "unexpected tag for teletext_descriptor",
        )?;
        if body.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "teletext_descriptor length must be a multiple of 5",
            });
        }
        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            let language_code = LangCode([chunk[0], chunk[1], chunk[2]]);
            let type_and_mag = chunk[LANG_LEN];
            let teletext_type = TeletextType::from_u8((type_and_mag >> 3) & 0x1F);
            let magazine_number = type_and_mag & 0x07;
            let page_number = chunk[LANG_LEN + 1];
            entries.push(TeletextEntry {
                language_code,
                teletext_type,
                magazine_number,
                page_number,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for TeletextDescriptor {
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
            buf[pos..pos + LANG_LEN].copy_from_slice(&e.language_code.0);
            buf[pos + LANG_LEN] =
                ((e.teletext_type.to_u8() & 0x1F) << 3) | (e.magazine_number & 0x07);
            buf[pos + LANG_LEN + 1] = e.page_number;
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for TeletextDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TELETEXT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        // lang=eng, type=1, mag=2, page=0x10
        let bytes = [TAG, 5, b'e', b'n', b'g', (1 << 3) | 2, 0x10];
        let d = TeletextDescriptor::parse(&bytes).unwrap();
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
        let d = TeletextDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].teletext_type, TeletextType::SubtitlePage);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            TeletextDescriptor::parse(&[0x57, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x57, .. }
        ));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_5() {
        let bytes = [TAG, 4, 0, 0, 0, 0];
        assert!(matches!(
            TeletextDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = TeletextDescriptor {
            entries: vec![TeletextEntry {
                language_code: LangCode(*b"fra"),
                teletext_type: TeletextType::SubtitlePage,
                magazine_number: 8 & 0x07,
                page_number: 0x88,
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(TeletextDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn empty_descriptor_valid() {
        let bytes = [TAG, 0];
        let d = TeletextDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 0);
    }

    #[test]
    fn teletext_type_full_range_round_trip() {
        for b in 0..=0xFF_u8 {
            let tt = TeletextType::from_u8(b);
            assert_eq!(tt.to_u8(), b, "round-trip failed for byte 0x{b:02X}");
        }
    }

    #[test]
    fn teletext_type_name_for_known() {
        assert_eq!(TeletextType::InitialPage.name(), "initial teletext page");
        assert_eq!(
            TeletextType::HearingImpairedSubtitlePage.name(),
            "teletext subtitle page for hearing impaired people"
        );
        assert_eq!(TeletextType::Reserved(0x06).name(), "reserved");
    }
}
