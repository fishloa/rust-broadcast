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

/// Subtitling type — ETSI EN 300 468 Table 26 (`stream_content = 0x03`).
///
/// Wire values `0x10`–`0x2F` are defined per §6.2.41 / Table 26.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SubtitlingType {
    /// 0x01 — EBU teletext subtitles.
    EbuTeletextSubtitles,
    /// 0x02 — associated EBU teletext.
    AssociatedEbuTeletext,
    /// 0x03 — VBI data.
    VbiData,
    /// 0x10 — DVB subtitles (normal) with no monitor aspect ratio critical.
    DvbSubtitlesNormal,
    /// 0x11 — DVB subtitles (normal) for display on 4:3 aspect ratio monitor.
    DvbSubtitlesNormal4x3,
    /// 0x12 — DVB subtitles (normal) for display on 16:9 aspect ratio monitor.
    DvbSubtitlesNormal16x9,
    /// 0x13 — DVB subtitles (normal) for display on 2.21:1 aspect ratio monitor.
    DvbSubtitlesNormal2p21x1,
    /// 0x14 — DVB subtitles (normal) for display on a high definition monitor.
    DvbSubtitlesNormalHd,
    /// 0x15 — DVB subtitles (normal), plano-stereoscopic disparity, HD.
    DvbSubtitlesNormalPlanoStereoscopicHd,
    /// 0x16 — DVB subtitles (normal) for display on an ultra high definition
    /// monitor.
    DvbSubtitlesNormalUhd,
    /// 0x20 — DVB subtitles (hard of hearing) with no monitor aspect ratio
    /// critical.
    DvbSubtitlesHardOfHearing,
    /// 0x21 — DVB subtitles (hard of hearing) for display on 4:3 aspect ratio
    /// monitor.
    DvbSubtitlesHardOfHearing4x3,
    /// 0x22 — DVB subtitles (hard of hearing) for display on 16:9 aspect ratio
    /// monitor.
    DvbSubtitlesHardOfHearing16x9,
    /// 0x23 — DVB subtitles (hard of hearing) for display on 2.21:1 aspect ratio
    /// monitor.
    DvbSubtitlesHardOfHearing2p21x1,
    /// 0x24 — DVB subtitles (hard of hearing) for display on a high definition
    /// monitor.
    DvbSubtitlesHardOfHearingHd,
    /// 0x25 — DVB subtitles (hard of hearing), plano-stereoscopic disparity, HD.
    DvbSubtitlesHardOfHearingPlanoStereoscopicHd,
    /// 0x26 — DVB subtitles (hard of hearing) for display on an ultra high
    /// definition monitor.
    DvbSubtitlesHardOfHearingUhd,
    /// 0x30 — open (in-vision) sign language interpretation for the deaf.
    OpenSignLanguage,
    /// 0x31 — closed sign language interpretation for the deaf.
    ClosedSignLanguage,
    /// Reserved/unallocated wire value, preserved verbatim for round-trip.
    Reserved(u8),
}

impl SubtitlingType {
    #[must_use]
    /// Creates a value from a wire byte, preserving every possible
    /// byte value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::EbuTeletextSubtitles,
            0x02 => Self::AssociatedEbuTeletext,
            0x03 => Self::VbiData,
            0x10 => Self::DvbSubtitlesNormal,
            0x11 => Self::DvbSubtitlesNormal4x3,
            0x12 => Self::DvbSubtitlesNormal16x9,
            0x13 => Self::DvbSubtitlesNormal2p21x1,
            0x14 => Self::DvbSubtitlesNormalHd,
            0x15 => Self::DvbSubtitlesNormalPlanoStereoscopicHd,
            0x16 => Self::DvbSubtitlesNormalUhd,
            0x20 => Self::DvbSubtitlesHardOfHearing,
            0x21 => Self::DvbSubtitlesHardOfHearing4x3,
            0x22 => Self::DvbSubtitlesHardOfHearing16x9,
            0x23 => Self::DvbSubtitlesHardOfHearing2p21x1,
            0x24 => Self::DvbSubtitlesHardOfHearingHd,
            0x25 => Self::DvbSubtitlesHardOfHearingPlanoStereoscopicHd,
            0x26 => Self::DvbSubtitlesHardOfHearingUhd,
            0x30 => Self::OpenSignLanguage,
            0x31 => Self::ClosedSignLanguage,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the wire byte for this value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::EbuTeletextSubtitles => 0x01,
            Self::AssociatedEbuTeletext => 0x02,
            Self::VbiData => 0x03,
            Self::DvbSubtitlesNormal => 0x10,
            Self::DvbSubtitlesNormal4x3 => 0x11,
            Self::DvbSubtitlesNormal16x9 => 0x12,
            Self::DvbSubtitlesNormal2p21x1 => 0x13,
            Self::DvbSubtitlesNormalHd => 0x14,
            Self::DvbSubtitlesNormalPlanoStereoscopicHd => 0x15,
            Self::DvbSubtitlesNormalUhd => 0x16,
            Self::DvbSubtitlesHardOfHearing => 0x20,
            Self::DvbSubtitlesHardOfHearing4x3 => 0x21,
            Self::DvbSubtitlesHardOfHearing16x9 => 0x22,
            Self::DvbSubtitlesHardOfHearing2p21x1 => 0x23,
            Self::DvbSubtitlesHardOfHearingHd => 0x24,
            Self::DvbSubtitlesHardOfHearingPlanoStereoscopicHd => 0x25,
            Self::DvbSubtitlesHardOfHearingUhd => 0x26,
            Self::OpenSignLanguage => 0x30,
            Self::ClosedSignLanguage => 0x31,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns a human-readable spec name for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::EbuTeletextSubtitles => "EBU teletext subtitles",
            Self::AssociatedEbuTeletext => "associated EBU teletext",
            Self::VbiData => "VBI data",
            Self::DvbSubtitlesNormal => "DVB subtitles (normal), no aspect ratio critical",
            Self::DvbSubtitlesNormal4x3 => "DVB subtitles (normal), 4:3",
            Self::DvbSubtitlesNormal16x9 => "DVB subtitles (normal), 16:9",
            Self::DvbSubtitlesNormal2p21x1 => "DVB subtitles (normal), 2.21:1",
            Self::DvbSubtitlesNormalHd => "DVB subtitles (normal), HD",
            Self::DvbSubtitlesNormalPlanoStereoscopicHd => {
                "DVB subtitles (normal), plano-stereoscopic disparity, HD"
            }
            Self::DvbSubtitlesNormalUhd => "DVB subtitles (normal), UHD",
            Self::DvbSubtitlesHardOfHearing => {
                "DVB subtitles (hard of hearing), no aspect ratio critical"
            }
            Self::DvbSubtitlesHardOfHearing4x3 => "DVB subtitles (hard of hearing), 4:3",
            Self::DvbSubtitlesHardOfHearing16x9 => "DVB subtitles (hard of hearing), 16:9",
            Self::DvbSubtitlesHardOfHearing2p21x1 => "DVB subtitles (hard of hearing), 2.21:1",
            Self::DvbSubtitlesHardOfHearingHd => "DVB subtitles (hard of hearing), HD",
            Self::DvbSubtitlesHardOfHearingPlanoStereoscopicHd => {
                "DVB subtitles (hard of hearing), plano-stereoscopic disparity, HD"
            }
            Self::DvbSubtitlesHardOfHearingUhd => "DVB subtitles (hard of hearing), UHD",
            Self::OpenSignLanguage => "open (in-vision) sign language interpretation",
            Self::ClosedSignLanguage => "closed sign language interpretation",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// One subtitling component.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubtitlingEntry {
    /// ISO 639-2 language code.
    pub language_code: LangCode,
    /// subtitling_type (ETSI EN 300 468 Table 26, `stream_content = 0x03`).
    pub subtitling_type: SubtitlingType,
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
                subtitling_type: SubtitlingType::from_u8(chunk[3]),
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
            buf[pos + 3] = e.subtitling_type.to_u8();
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
        assert_eq!(
            d.entries[0].subtitling_type,
            SubtitlingType::DvbSubtitlesNormal
        );
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
                    subtitling_type: SubtitlingType::DvbSubtitlesNormal,
                    composition_page_id: 0x1234,
                    ancillary_page_id: 0x5678,
                },
                SubtitlingEntry {
                    language_code: LangCode(*b"deu"),
                    subtitling_type: SubtitlingType::DvbSubtitlesHardOfHearing,
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

    #[test]
    fn subtitling_type_full_range_round_trip() {
        for b in 0..=0xFF_u8 {
            let st = SubtitlingType::from_u8(b);
            assert_eq!(st.to_u8(), b, "round-trip failed for byte 0x{b:02X}");
        }
    }

    #[test]
    fn subtitling_type_name_for_known() {
        assert_eq!(
            SubtitlingType::EbuTeletextSubtitles.name(),
            "EBU teletext subtitles"
        );
        assert_eq!(
            SubtitlingType::DvbSubtitlesNormal.name(),
            "DVB subtitles (normal), no aspect ratio critical"
        );
        assert_eq!(
            SubtitlingType::DvbSubtitlesNormalUhd.name(),
            "DVB subtitles (normal), UHD"
        );
        assert_eq!(
            SubtitlingType::DvbSubtitlesHardOfHearingUhd.name(),
            "DVB subtitles (hard of hearing), UHD"
        );
        assert_eq!(
            SubtitlingType::OpenSignLanguage.name(),
            "open (in-vision) sign language interpretation"
        );
        assert_eq!(
            SubtitlingType::ClosedSignLanguage.name(),
            "closed sign language interpretation"
        );
        assert_eq!(SubtitlingType::Reserved(0x50).name(), "reserved");
    }

    #[test]
    fn subtitling_type_round_trip_0x15_0x25() {
        assert_eq!(SubtitlingType::from_u8(0x15).to_u8(), 0x15);
        assert_eq!(SubtitlingType::from_u8(0x25).to_u8(), 0x25);
    }
}
