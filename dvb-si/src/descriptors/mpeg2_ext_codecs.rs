//! MPEG-2 `Extension_descriptor` bodies for LCEVC and media service kind —
//! Rec. ITU-T H.222.0 (08/2023) | ISO/IEC 13818-1:2023:
//!
//! * 0x17 `LCEVC_video_descriptor()` — §2.6.137, Table 2-142.
//! * 0x18 `LCEVC_linkage_descriptor()` — §2.6.139, Table 2-143.
//! * 0x19 `Media_service_kind_descriptor()` — §2.6.141, Table 2-144 (+ Tables
//!   2-145…2-151).
//!
//! Syntax transcription: `docs/descriptors/iso_13818_1/3f-extension-descriptor-bodies-0x0f-0x19.md`.
//! Every `reserved` field is a typed field re-emitted verbatim (default: all
//! ones; `reserved_zero_2bit` defaults to zero as its name says).

use super::hdr_wcg_idc::HdrWcgIdc;
use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter, invalid};
use alloc::vec::Vec;

const TAG: u8 = super::mpeg2_extension::TAG;
const RES1: u8 = 0x01;
const RES3: u8 = 0x07;

// ───────────────────────── 0x17 LCEVC_video_descriptor ──────────────────────

/// `LCEVC_video_descriptor()` (ext 0x17) — Table 2-142.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LcevcVideo {
    /// `lcevc_stream_tag`: association between base and enhancement streams.
    pub lcevc_stream_tag: u8,
    /// `profile_idc` (4 bits, ISO/IEC 23094-2 §7.3.4).
    pub profile_idc: u8,
    /// `level_idc` (4 bits).
    pub level_idc: u8,
    /// `sublevel_idc` (2 bits).
    pub sublevel_idc: u8,
    /// `processed_planes_type_flag`.
    pub processed_planes_type_flag: bool,
    /// `picture_type_bit_flag`.
    pub picture_type_bit_flag: bool,
    /// `field_type_bit_flag`.
    pub field_type_bit_flag: bool,
    /// `reserved(3)`, default 7.
    pub reserved: u8,
    /// `HDR_WCG_idc` (Table 2-114).
    pub hdr_wcg_idc: HdrWcgIdc,
    /// `reserved_zero_2bit`, default 0.
    pub reserved_zero_2bit: u8,
    /// `video_properties_tag` (4 bits; Tables 2-135…2-138 select its meaning by
    /// `HDR_WCG_idc`).
    pub video_properties_tag: u8,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl LcevcVideo {
    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "LcevcVideo");
        Ok(Self {
            lcevc_stream_tag: r.u8(8)?,
            profile_idc: r.u8(4)?,
            level_idc: r.u8(4)?,
            sublevel_idc: r.u8(2)?,
            processed_planes_type_flag: r.flag()?,
            picture_type_bit_flag: r.flag()?,
            field_type_bit_flag: r.flag()?,
            reserved: r.u8(3)?,
            hdr_wcg_idc: HdrWcgIdc::from_u8(r.u8(2)?),
            reserved_zero_2bit: r.u8(2)?,
            video_properties_tag: r.u8(4)?,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(u64::from(self.lcevc_stream_tag), 8, "lcevc_stream_tag")?;
        w.put(u64::from(self.profile_idc), 4, "profile_idc")?;
        w.put(u64::from(self.level_idc), 4, "level_idc")?;
        w.put(u64::from(self.sublevel_idc), 2, "sublevel_idc")?;
        w.flag(
            self.processed_planes_type_flag,
            "processed_planes_type_flag",
        )?;
        w.flag(self.picture_type_bit_flag, "picture_type_bit_flag")?;
        w.flag(self.field_type_bit_flag, "field_type_bit_flag")?;
        w.put(u64::from(self.reserved), 3, "reserved")?;
        w.put(u64::from(self.hdr_wcg_idc.to_u8()), 2, "HDR_WCG_idc")?;
        w.put(u64::from(self.reserved_zero_2bit), 2, "reserved_zero_2bit")?;
        w.put(
            u64::from(self.video_properties_tag),
            4,
            "video_properties_tag",
        )?;
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

impl LcevcVideo {
    /// Default of `reserved(3)` (all ones).
    pub const RESERVED_DEFAULT: u8 = RES3;
}

// ───────────────────────── 0x18 LCEVC_linkage_descriptor ────────────────────

/// `LCEVC_linkage_descriptor()` (ext 0x18) — Table 2-143.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LcevcLinkage {
    /// `lcevc_stream_tag[i]` (`num_lcevc_stream_tags` is 8 bits).
    pub lcevc_stream_tags: Vec<u8>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl LcevcLinkage {
    /// Build with no trailing bytes.
    #[must_use]
    pub fn new(lcevc_stream_tags: Vec<u8>) -> Self {
        Self {
            lcevc_stream_tags,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "LcevcLinkage");
        let n = r.bits(8)?;
        let mut tags = Vec::new();
        for _ in 0..n {
            tags.push(r.u8(8)?);
        }
        Ok(Self {
            lcevc_stream_tags: tags,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(
            self.lcevc_stream_tags.len() as u64,
            8,
            "num_lcevc_stream_tags",
        )?;
        w.bytes(&self.lcevc_stream_tags);
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

// ───────────────────────── 0x19 Media_service_kind_descriptor ───────────────

/// `media_type_idc` (Table 2-146).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MediaTypeIdc {
    /// 0 — unknown.
    Unknown,
    /// 1 — video.
    Video,
    /// 2 — audio.
    Audio,
    /// 3 — text/data.
    TextData,
    /// Out-of-range value (cannot occur from a 2-bit field), carried verbatim.
    Reserved(u8),
}

impl MediaTypeIdc {
    /// Decode the 2-bit field.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Unknown,
            1 => Self::Video,
            2 => Self::Audio,
            3 => Self::TextData,
            other => Self::Reserved(other),
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Video => 1,
            Self::Audio => 2,
            Self::TextData => 3,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (Table 2-146).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::TextData => "text/data",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(MediaTypeIdc, Reserved);

/// `configuration_type` (Table 2-149).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ConfigurationType {
    /// 0 — complete: a single playable experience.
    Complete,
    /// 1 — partial: not playable alone.
    Partial,
    /// 2 — complete combination: several playable experiences.
    CompleteCombination,
    /// 3 — reserved (carried verbatim).
    Reserved(u8),
}

impl ConfigurationType {
    /// Decode the 2-bit field.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Complete,
            1 => Self::Partial,
            2 => Self::CompleteCombination,
            other => Self::Reserved(other),
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Complete => 0,
            Self::Partial => 1,
            Self::CompleteCombination => 2,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (Table 2-149).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::CompleteCombination => "complete combination",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(ConfigurationType, Reserved);

/// `ID_length_code` 7: the length is carried explicitly in `ID_len`.
pub const ID_LENGTH_CODE_EXPLICIT: u8 = 7;
/// Table 2-147: `ID_len` in bytes for `ID_length_code` 0..=6.
pub const ID_LENGTHS: [usize; 7] = [1, 2, 4, 8, 12, 16, 20];
/// `lang_len_idc` 0: explicit `lang_len` byte.
const LANG_LEN_EXPLICIT: u8 = 0;

/// The optional identifier of a media service kind entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaIdentifier {
    /// `ID_length_code` (3 bits, Table 2-147); on serialize it must agree with
    /// `media_id.len()` (7 = explicit `ID_len`), so a non-canonical choice made
    /// by the sender survives a round trip.
    pub id_length_code: u8,
    /// `ID_type` (13 bits, Table 2-148).
    pub id_type: u16,
    /// `media_ID_field` (`ID_len` bytes).
    pub media_id: Vec<u8>,
}

/// One language / purposes pair of a media service kind entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaServiceLanguage {
    /// `configuration_type` (Table 2-149).
    pub configuration_type: ConfigurationType,
    /// `lang_len_idc` (2 bits, Table 2-150: 0 explicit `lang_len`, 1 → 2 bytes,
    /// 2 → 3 bytes, 3 reserved); must agree with `language_code.len()` on serialize.
    pub lang_len_idc: u8,
    /// `reserved(1)` after `lang_len_idc`, default 1.
    pub reserved: u8,
    /// `IETF_BCP_47_language_code` (`lang_len` bytes, ISO 8859-1 characters).
    pub language_code: Vec<u8>,
    /// `media_service_type` values (`lang_purpose_cnt` of them, 3 bits; 8-bit
    /// codes whose meaning depends on `media_type_idc`, Tables 2-151…2-153).
    pub media_service_types: Vec<u8>,
}

/// One data element (outer loop) of the media service kind descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaServiceKindEntry {
    /// `media_description_flag` (Table 2-145: false = self, true = associate).
    pub media_description_flag: bool,
    /// `media_type_idc` (Table 2-146).
    pub media_type_idc: MediaTypeIdc,
    /// `reserved(1)` after `media_type_idc`, default 1.
    pub reserved: u8,
    /// The identifier, present iff `identifier_flag`.
    pub identifier: Option<MediaIdentifier>,
    /// The `lang_pairs` language entries (3-bit count).
    pub languages: Vec<MediaServiceLanguage>,
}

impl MediaServiceKindEntry {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        media_description_flag: bool,
        media_type_idc: MediaTypeIdc,
        identifier: Option<MediaIdentifier>,
        languages: Vec<MediaServiceLanguage>,
    ) -> Self {
        Self {
            media_description_flag,
            media_type_idc,
            reserved: RES1,
            identifier,
            languages,
        }
    }
}

impl MediaServiceLanguage {
    /// Build with `lang_len_idc` chosen from the code length (2 → 1, 3 → 2,
    /// otherwise explicit) and the spec `reserved` fill.
    #[must_use]
    pub fn new(
        configuration_type: ConfigurationType,
        language_code: Vec<u8>,
        media_service_types: Vec<u8>,
    ) -> Self {
        let lang_len_idc = match language_code.len() {
            2 => 1,
            3 => 2,
            _ => LANG_LEN_EXPLICIT,
        };
        Self {
            configuration_type,
            lang_len_idc,
            reserved: RES1,
            language_code,
            media_service_types,
        }
    }
}

/// `Media_service_kind_descriptor()` (ext 0x19) — Table 2-144.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaServiceKind {
    /// The data elements (`N` is implied by the descriptor size).
    pub entries: Vec<MediaServiceKindEntry>,
}

impl MediaServiceKind {
    /// Wrap the entries.
    #[must_use]
    pub fn new(entries: Vec<MediaServiceKindEntry>) -> Self {
        Self { entries }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "MediaServiceKind");
        let mut entries = Vec::new();
        while r.remaining_bytes() > 0 {
            let media_description_flag = r.flag()?;
            let identifier_flag = r.flag()?;
            let lang_pairs = r.bits(3)?;
            let media_type_idc = MediaTypeIdc::from_u8(r.u8(2)?);
            let reserved = r.u8(1)?;
            let identifier = if identifier_flag {
                let id_length_code = r.u8(3)?;
                let id_type = r.u16(13)?;
                let len = if id_length_code == ID_LENGTH_CODE_EXPLICIT {
                    usize::from(r.u8(8)?)
                } else {
                    ID_LENGTHS[usize::from(id_length_code)]
                };
                Some(MediaIdentifier {
                    id_length_code,
                    id_type,
                    media_id: r.bytes(len)?.to_vec(),
                })
            } else {
                None
            };
            let mut languages = Vec::new();
            for _ in 0..lang_pairs {
                let configuration_type = ConfigurationType::from_u8(r.u8(2)?);
                let purpose_cnt = r.bits(3)?;
                let lang_len_idc = r.u8(2)?;
                let reserved = r.u8(1)?;
                let len = match lang_len_idc {
                    0 => usize::from(r.u8(8)?),
                    1 => 2,
                    2 => 3,
                    _ => {
                        return Err(invalid(
                            TAG,
                            "Media_service_kind_descriptor lang_len_idc 3 is reserved",
                        ));
                    }
                };
                let language_code = r.bytes(len)?.to_vec();
                let media_service_types = r.bytes(purpose_cnt as usize)?.to_vec();
                languages.push(MediaServiceLanguage {
                    configuration_type,
                    lang_len_idc,
                    reserved,
                    language_code,
                    media_service_types,
                });
            }
            entries.push(MediaServiceKindEntry {
                media_description_flag,
                media_type_idc,
                reserved,
                identifier,
                languages,
            });
        }
        Ok(Self { entries })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        for e in &self.entries {
            w.flag(e.media_description_flag, "media_description_flag")?;
            w.flag(e.identifier.is_some(), "identifier_flag")?;
            w.put(e.languages.len() as u64, 3, "lang_pairs")?;
            w.put(u64::from(e.media_type_idc.to_u8()), 2, "media_type_idc")?;
            w.put(u64::from(e.reserved), 1, "reserved")?;
            if let Some(id) = &e.identifier {
                w.put(u64::from(id.id_length_code), 3, "ID_length_code")?;
                w.put(u64::from(id.id_type), 13, "ID_type")?;
                if id.id_length_code == ID_LENGTH_CODE_EXPLICIT {
                    w.put(id.media_id.len() as u64, 8, "ID_len")?;
                } else if ID_LENGTHS.get(usize::from(id.id_length_code)) != Some(&id.media_id.len())
                {
                    return Err(invalid(
                        TAG,
                        "Media_service_kind_descriptor media_ID_field length disagrees with ID_length_code",
                    ));
                }
                w.bytes(&id.media_id);
            }
            for l in &e.languages {
                w.put(
                    u64::from(l.configuration_type.to_u8()),
                    2,
                    "configuration_type",
                )?;
                w.put(l.media_service_types.len() as u64, 3, "lang_purpose_cnt")?;
                w.put(u64::from(l.lang_len_idc), 2, "lang_len_idc")?;
                w.put(u64::from(l.reserved), 1, "reserved")?;
                match (l.lang_len_idc, l.language_code.len()) {
                    (0, n) => w.put(n as u64, 8, "lang_len")?,
                    (1, 2) | (2, 3) => {}
                    _ => {
                        return Err(invalid(
                            TAG,
                            "Media_service_kind_descriptor language code length disagrees with lang_len_idc",
                        ));
                    }
                }
                w.bytes(&l.language_code);
                w.bytes(&l.media_service_types);
            }
        }
        Ok(w.into_vec())
    }
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;

    #[test]
    fn lcevc_video_spec_vector() {
        // tag=7, profile=3 level=5, sublevel=1 planes=1 pic=0 field=1 reserved=111,
        // HDR_WCG=2 reserved_zero=00 vpt=2
        let b = [0x07, 0x35, 0b01_1_0_1_111, 0b10_00_0010];
        let d = LcevcVideo::parse_body(&b).unwrap();
        assert_eq!((d.profile_idc, d.level_idc, d.sublevel_idc), (3, 5, 1));
        assert!(d.processed_planes_type_flag && !d.picture_type_bit_flag && d.field_type_bit_flag);
        assert_eq!(d.hdr_wcg_idc, HdrWcgIdc::HdrAndWcg);
        assert_eq!(d.video_properties_tag, 2);
        assert_eq!(d.write_body().unwrap(), b);
        let mut bad = d.clone();
        bad.profile_idc = 16;
        assert!(bad.write_body().is_err());
        assert!(LcevcVideo::parse_body(&b[..3]).is_err());
    }

    #[test]
    fn lcevc_linkage_vectors() {
        let b = [2, 0x10, 0x20, 0xFF];
        let d = LcevcLinkage::parse_body(&b).unwrap();
        assert_eq!(d.lcevc_stream_tags, [0x10, 0x20]);
        assert_eq!(d.trailing, [0xFF]);
        assert_eq!(d.write_body().unwrap(), b);
        assert!(LcevcLinkage::parse_body(&[3, 1]).is_err());
        assert!(LcevcLinkage::new(vec![0; 256]).write_body().is_err());
    }

    #[test]
    fn media_service_kind_round_trip() {
        let id = MediaIdentifier {
            id_length_code: 2,
            id_type: 0x0200,
            media_id: vec![1, 2, 3, 4],
        };
        let l1 = MediaServiceLanguage::new(ConfigurationType::Complete, b"en".to_vec(), vec![1, 9]);
        let l2 = MediaServiceLanguage::new(ConfigurationType::Partial, b"zxx-x".to_vec(), vec![]);
        let e1 = MediaServiceKindEntry::new(false, MediaTypeIdc::Audio, Some(id), vec![l1, l2]);
        let e2 = MediaServiceKindEntry::new(true, MediaTypeIdc::Video, None, vec![]);
        let d = MediaServiceKind::new(vec![e1, e2]);
        let b = d.write_body().unwrap();
        // entry 1: desc=0 ident=1 pairs=2 type=2 reserved=1
        assert_eq!(b[0], 0b0_1_010_10_1);
        // ID_length_code=2 (4 bytes), ID_type=0x0200
        assert_eq!(&b[1..3], &[0b010_00010, 0x00]);
        assert_eq!(&b[3..7], &[1, 2, 3, 4]);
        // pair 1: config=0, cnt=2, idc=1, reserved=1
        assert_eq!(b[7], 0b00_010_01_1);
        assert_eq!(&b[8..12], &[b'e', b'n', 1, 9]);
        // pair 2: config=1, cnt=0, idc=0 explicit, reserved=1 then lang_len=5
        assert_eq!(&b[12..14], &[0b01_000_00_1, 5]);
        assert_eq!(MediaServiceKind::parse_body(&b).unwrap(), d);
    }

    #[test]
    fn media_service_kind_rejects_inconsistent_and_reserved() {
        let mut l = MediaServiceLanguage::new(ConfigurationType::Complete, b"en".to_vec(), vec![]);
        l.lang_len_idc = 2; // claims 3 bytes
        let e = MediaServiceKindEntry::new(false, MediaTypeIdc::Audio, None, vec![l]);
        assert!(MediaServiceKind::new(vec![e]).write_body().is_err());
        // lang_len_idc = 3 is reserved
        let b = [0b0_0_001_01_1, 0b00_000_11_1];
        assert!(MediaServiceKind::parse_body(&b).is_err());
        // identifier length disagrees with ID_length_code
        let id = MediaIdentifier {
            id_length_code: 0,
            id_type: 1,
            media_id: vec![1, 2],
        };
        let e = MediaServiceKindEntry::new(false, MediaTypeIdc::Audio, Some(id), vec![]);
        assert!(MediaServiceKind::new(vec![e]).write_body().is_err());
    }
}
