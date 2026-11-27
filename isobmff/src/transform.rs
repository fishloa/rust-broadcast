//! Protected and incomplete sample-entry transformations —
//! ISO/IEC 14496-12:2026 §13.4.2 (Table 13) and §13.6.2.
//!
//! A protected entry replaces the original four-CC with `enc*` and adds a
//! `ProtectionSchemeInfoBox` (`sinf`); an incomplete entry replaces it with
//! `icp*` and adds a `CompleteTrackInfoBox` (`cinf`). In both cases every other
//! field and box is left unmodified, so this module keeps the base class's
//! fixed bytes and the appended transform box verbatim and exposes accessors.
//!
//! The `encv`/`enca` video and audio codes keep their own fully typed entries;
//! this module types the remaining media classes whose base adds no field of
//! its own (font, metadata, haptics, system) and the volumetric class (a
//! 32-byte `compressorname`).

use alloc::vec::Vec;
use broadcast_core::len::need_out;
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, box_len, compact_form, parse_box, write_box_header,
};
use crate::error::{Error, Result};
use crate::sample_entries::SAMPLE_ENTRY_RESERVED_LEN;

/// `encf`: protected font entry (§13.4.2 Table 13).
pub const ENCF: [u8; BOX_TYPE_SIZE] = *b"encf";
/// `encm`: protected metadata entry (§13.4.2 Table 13).
pub const ENCM: [u8; BOX_TYPE_SIZE] = *b"encm";
/// `encp`: protected haptics entry (§13.4.2 Table 13).
pub const ENCP: [u8; BOX_TYPE_SIZE] = *b"encp";
/// `encs`: protected system entry (§13.4.2 Table 13).
pub const ENCS: [u8; BOX_TYPE_SIZE] = *b"encs";
/// `enc3`: protected volumetric visual entry (§13.4.2 Table 13).
pub const ENC3: [u8; BOX_TYPE_SIZE] = *b"enc3";
/// `icps`: incomplete system entry (§13.6.2).
pub const ICPS: [u8; BOX_TYPE_SIZE] = *b"icps";
/// `icpp`: incomplete haptics entry (§13.6.2).
pub const ICPP: [u8; BOX_TYPE_SIZE] = *b"icpp";
/// `icpm`: incomplete timed-metadata entry (§13.6.2).
pub const ICPM: [u8; BOX_TYPE_SIZE] = *b"icpm";
/// `icp3`: incomplete volumetric visual entry (§13.6.2).
pub const ICP3: [u8; BOX_TYPE_SIZE] = *b"icp3";
/// `icpv`: incomplete video entry (§13.6.2).
pub const ICPV: [u8; BOX_TYPE_SIZE] = *b"icpv";

/// `SampleEntry` base: `reserved[6]` + `data_reference_index`.
const BASE_PLAIN_LEN: usize = SAMPLE_ENTRY_RESERVED_LEN + core::mem::size_of::<u16>();
/// The volumetric visual base adds an 8-bit `compressorname[32]` (§12.1.3).
const COMPRESSOR_NAME_LEN: usize = 32;
/// `revision_level` + `vendor` width in an audio entry's fixed part.
const AUDIO_VENDOR_BLOCK_LEN: usize = 6;
/// `samplesize` an empty audio entry states.
const DEFAULT_SAMPLESIZE: u16 = 16;

/// The fixed part a transformed entry keeps unmodified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TransformBase {
    /// The plain `SampleEntry` base (font, metadata, haptics, system).
    Sample,
    /// The volumetric visual base: the plain base plus `compressorname[32]`.
    Volumetric,
    /// The `VisualSampleEntry` base (78 bytes, §12.1.3).
    Visual,
}

impl TransformBase {
    /// Bytes of the base class's fixed part.
    #[must_use]
    pub const fn fixed_len(self) -> usize {
        match self {
            Self::Sample => BASE_PLAIN_LEN,
            Self::Volumetric => BASE_PLAIN_LEN + COMPRESSOR_NAME_LEN,
            Self::Visual => crate::sample_entries::VisualSampleEntryFields::serialized_len(),
        }
    }
}

/// One code this module types: its base class and transform kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EntryKind {
    base: TransformBase,
    protected: bool,
}

const fn kind_of(codec_type: [u8; BOX_TYPE_SIZE]) -> Option<EntryKind> {
    let (base, protected) = match &codec_type {
        b"encf" | b"encm" | b"encp" | b"encs" => (TransformBase::Sample, true),
        b"enc3" => (TransformBase::Volumetric, true),
        b"icps" | b"icpp" | b"icpm" => (TransformBase::Sample, false),
        b"icp3" => (TransformBase::Volumetric, false),
        b"icpv" => (TransformBase::Visual, false),
        _ => return None,
    };
    Some(EntryKind { base, protected })
}

/// A protected or incomplete sample entry whose base class adds no field of
/// its own beyond [`TransformBase`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TransformSampleEntry {
    codec_type: [u8; BOX_TYPE_SIZE],
    base: TransformBase,
    fixed: Vec<u8>,
    transform: Option<OpaqueBox>,
    other_boxes: Vec<OpaqueBox>,
    trailing: Vec<u8>,
}

impl TransformSampleEntry {
    /// Build one with a zeroed base (data reference index 1) and no children.
    ///
    /// # Errors
    /// `codec_type` is not one this module types.
    pub fn new(codec_type: [u8; BOX_TYPE_SIZE]) -> Result<Self> {
        let kind = kind_of(codec_type).ok_or(Error::UnexpectedBox {
            expected: "a protected or incomplete sample entry",
        })?;
        let mut fixed = alloc::vec![0u8; kind.base.fixed_len()];
        if let Some(dri) = fixed.get_mut(SAMPLE_ENTRY_RESERVED_LEN..BASE_PLAIN_LEN) {
            dri.copy_from_slice(&1u16.to_be_bytes());
        }
        Ok(Self {
            codec_type,
            base: kind.base,
            fixed,
            transform: None,
            other_boxes: Vec::new(),
            trailing: Vec::new(),
        })
    }

    /// The entry's four-CC.
    #[must_use]
    pub const fn codec_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.codec_type
    }

    /// The base class.
    #[must_use]
    pub const fn base(&self) -> TransformBase {
        self.base
    }

    /// The base class's fixed bytes, verbatim.
    #[must_use]
    pub fn fixed(&self) -> &[u8] {
        &self.fixed
    }

    /// The `VisualSampleEntry` fields, when this entry's base is visual.
    #[must_use]
    pub fn visual(&self) -> Option<crate::sample_entries::VisualSampleEntryFields> {
        if self.base != TransformBase::Visual {
            return None;
        }
        let mut whole = alloc::vec![0u8; BOX_HEADER_MIN_SIZE + self.fixed.len()];
        let size = broadcast_core::len::fit_u32(whole.len(), "icpv").ok()?;
        whole[..core::mem::size_of::<u32>()].copy_from_slice(&size.to_be_bytes());
        whole[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE].copy_from_slice(&self.codec_type);
        whole[BOX_HEADER_MIN_SIZE..].copy_from_slice(&self.fixed);
        crate::sample_entries::VisualSampleEntryFields::parse_body(&whole, "icpv").ok()
    }

    /// The transform box's payload, verbatim (its four-CC is `sinf` or `cinf`).
    #[must_use]
    pub fn transform_payload(&self) -> Option<&[u8]> {
        self.transform.as_ref().map(|box_| box_.data.as_slice())
    }

    /// The protection scheme info (`sinf`, §13.4.3), when this is a protected
    /// entry.
    #[must_use]
    pub fn protection_scheme(&self) -> Option<crate::cenc::ProtectionSchemeInfoBox> {
        let box_ = self.transform.as_ref()?;
        let mut whole = alloc::vec![0u8; BOX_HEADER_MIN_SIZE + box_.data.len()];
        let size = broadcast_core::len::fit_u32(whole.len(), "sinf").ok()?;
        whole[..core::mem::size_of::<u32>()].copy_from_slice(&size.to_be_bytes());
        whole[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE].copy_from_slice(&box_.box_type);
        whole[BOX_HEADER_MIN_SIZE..].copy_from_slice(&box_.data);
        crate::cenc::ProtectionSchemeInfoBox::parse(&whole).ok()
    }

    /// The complete-track info (`cinf`, §13.6.3), when this is an incomplete
    /// entry.
    #[must_use]
    pub fn complete_track_info(&self) -> Option<crate::scheme::CompleteTrackInfoBox> {
        let box_ = self.transform.as_ref()?;
        let mut whole = alloc::vec![0u8; BOX_HEADER_MIN_SIZE + box_.data.len()];
        let size = broadcast_core::len::fit_u32(whole.len(), "cinf").ok()?;
        whole[..core::mem::size_of::<u32>()].copy_from_slice(&size.to_be_bytes());
        whole[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE].copy_from_slice(&box_.box_type);
        whole[BOX_HEADER_MIN_SIZE..].copy_from_slice(&box_.data);
        crate::scheme::CompleteTrackInfoBox::parse(&whole).ok()
    }

    /// Every other box after the base and the transform, verbatim.
    #[must_use]
    pub fn other_boxes(&self) -> &[OpaqueBox] {
        &self.other_boxes
    }

    /// Bytes after the last complete child.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let item = parse_box(bytes)?.0;
        let mut codec_type = [0u8; BOX_TYPE_SIZE];
        codec_type.copy_from_slice(&item.four_cc());
        let kind = kind_of(codec_type).ok_or(Error::UnexpectedBox {
            expected: "a protected or incomplete sample entry",
        })?;
        let body = crate::box_types::box_body(bytes, kind.base.fixed_len(), "transformed entry")?;
        let fixed = body[..kind.base.fixed_len()].to_vec();
        let region = &body[kind.base.fixed_len()..];
        let (captured, trailing) = crate::box_support::capture_children(region).into_parts();
        let mut transform = None;
        let mut other_boxes = Vec::new();
        let wanted = if kind.protected {
            crate::cenc::SINF
        } else {
            crate::scheme::CINF
        };
        for child in captured {
            if transform.is_none() && child.box_type == wanted {
                transform = Some(child);
            } else {
                other_boxes.push(child);
            }
        }
        Ok(Self {
            codec_type,
            base: kind.base,
            fixed,
            transform,
            other_boxes,
            trailing,
        })
    }
}

impl<'a> Parse<'a> for TransformSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for TransformSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + self.fixed.len() + self.trailing.len();
        if let Some(box_) = &self.transform {
            n += box_.serialized_len();
        }
        for child in &self.other_boxes {
            n += child.serialized_len();
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &self.codec_type, need)?;
        buf[c..c + self.fixed.len()].copy_from_slice(&self.fixed);
        c += self.fixed.len();
        if let Some(box_) = &self.transform {
            c += box_.serialize_into(&mut buf[c..])?;
        }
        for child in &self.other_boxes {
            c += child.serialize_into(&mut buf[c..])?;
        }
        buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(c + self.trailing.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_protected_font_entry_round_trips() {
        let sinf = OpaqueBox::new(
            crate::cenc::SINF,
            alloc::vec![0, 0, 0, 12, b'f', b'r', b'm', b'a'],
        );
        let mut entry = TransformSampleEntry::new(ENCF).expect("builds");
        entry.transform = Some(sinf);
        let mut out = alloc::vec![0u8; entry.serialized_len()];
        entry.serialize_into(&mut out).expect("serializes");
        let back = TransformSampleEntry::parse_box(&out).expect("parses");
        assert_eq!(back.codec_type(), ENCF);
        assert_eq!(back.base(), TransformBase::Sample);
        assert_eq!(back, entry);
    }

    #[test]
    fn an_incomplete_volumetric_entry_keeps_its_compressor_name() {
        let mut entry = TransformSampleEntry::new(ICP3).expect("builds");
        entry.fixed[BASE_PLAIN_LEN..].fill(0xA5);
        let mut out = alloc::vec![0u8; entry.serialized_len()];
        entry.serialize_into(&mut out).expect("serializes");
        let back = TransformSampleEntry::parse_box(&out).expect("parses");
        assert_eq!(back.base(), TransformBase::Volumetric);
        assert_eq!(
            &back.fixed()[BASE_PLAIN_LEN..],
            &[0xA5; COMPRESSOR_NAME_LEN]
        );
    }

    #[test]
    fn an_unrelated_type_is_refused() {
        assert!(TransformSampleEntry::new(*b"avc1").is_err());
    }
}

/// `icpa`: incomplete audio entry (§13.6.2).
pub const ICPA: [u8; BOX_TYPE_SIZE] = *b"icpa";

/// The incomplete audio sample entry (`icpa`) — §13.6.2.
///
/// Uses the same `AudioSampleEntry` fixed part as `resa` (version 0 or the
/// QuickTime 1/2 extension) and adds a `cinf` among its children.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct IncompleteAudioSampleEntry {
    entry: crate::init_segment::ParsedAudioSampleEntry,
}

impl IncompleteAudioSampleEntry {
    /// The entry's four-CC.
    pub const FOURCC: [u8; BOX_TYPE_SIZE] = ICPA;

    /// Build an empty entry with default fixed fields.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entry: crate::init_segment::ParsedAudioSampleEntry {
                reserved: [0; SAMPLE_ENTRY_RESERVED_LEN],
                entry_version: 0,
                reserved_1: [0; AUDIO_VENDOR_BLOCK_LEN],
                data_reference_index: 1,
                channelcount: 1,
                samplesize: DEFAULT_SAMPLESIZE,
                compression_id_and_packet_size: [0; core::mem::size_of::<u32>()],
                samplerate: 0,
                quicktime: None,
                config_boxes: Vec::new(),
                trailing: Vec::new(),
            },
        }
    }

    /// `channelcount`.
    #[must_use]
    pub const fn channelcount(&self) -> u16 {
        self.entry.channelcount
    }

    /// Every child box after the fixed part, `cinf` included.
    #[must_use]
    pub fn children(&self) -> &[OpaqueBox] {
        &self.entry.config_boxes
    }

    /// The complete-track info (`cinf`, §13.6.3), when present.
    #[must_use]
    pub fn complete_track_info(&self) -> Option<crate::scheme::CompleteTrackInfoBox> {
        let child = self
            .entry
            .config_boxes
            .iter()
            .find(|child| child.box_type == crate::scheme::CINF)?;
        let mut whole = alloc::vec![0u8; BOX_HEADER_MIN_SIZE + child.data.len()];
        let size = broadcast_core::len::fit_u32(whole.len(), "cinf").ok()?;
        whole[..core::mem::size_of::<u32>()].copy_from_slice(&size.to_be_bytes());
        whole[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE]
            .copy_from_slice(&crate::scheme::CINF);
        whole[BOX_HEADER_MIN_SIZE..].copy_from_slice(&child.data);
        crate::scheme::CompleteTrackInfoBox::parse(&whole).ok()
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let item = parse_box(bytes)?.0;
        if item.four_cc() != ICPA {
            return Err(Error::UnexpectedBox { expected: "icpa" });
        }
        Ok(Self {
            entry: crate::init_segment::parse_audio_sample_entry(bytes, "icpa")?,
        })
    }
}

impl Default for IncompleteAudioSampleEntry {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Parse<'a> for IncompleteAudioSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for IncompleteAudioSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::init_segment::audio_sample_entry_serialized_len(
            &self.entry.quicktime,
            &self.entry.config_boxes,
            &self.entry.trailing,
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        crate::init_segment::serialize_audio_sample_entry(
            buf,
            crate::init_segment::AudioSampleEntryFields {
                fourcc: &ICPA,
                reserved: self.entry.reserved,
                entry_version: self.entry.entry_version,
                reserved_1: self.entry.reserved_1,
                data_reference_index: self.entry.data_reference_index,
                channelcount: self.entry.channelcount,
                samplesize: self.entry.samplesize,
                compression_id_and_packet_size: self.entry.compression_id_and_packet_size,
                samplerate: self.entry.samplerate,
                quicktime: &self.entry.quicktime,
                config_boxes: &self.entry.config_boxes,
                trailing: &self.entry.trailing,
            },
        )
    }
}
