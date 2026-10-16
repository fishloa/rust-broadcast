//! MPEG-2 `Extension_descriptor` — Rec. ITU-T H.222.0 (2023) | ISO/IEC
//! 13818-1 clause 2.6.90, Table 2-109, tag **0x3F** (63); the descriptor's
//! second-level selector `extension_descriptor_tag` is listed in Table 2-110.
//!
//! This is the *MPEG-2 systems* extension mechanism (tag 0x3F, TS and PS), not
//! the DVB `extension_descriptor` (tag 0x7F, EN 300 468 §6.2.16,
//! [`super::extension`]).
//!
//! Typed bodies: MPEG-H 3D audio (0x08…0x0E, [`super::mpegh_3d_audio`]),
//! the timing/HRD ones (0x03, 0x15, 0x16, [`super::h222_timing`]), the
//! layered/tiled HEVC ones (0x05, 0x06, 0x12, 0x13, [`super::mpeg2_ext_hevc`]),
//! the LCEVC / media service kind ones (0x17…0x19, [`super::mpeg2_ext_codecs`])
//! and 0x02, 0x04, 0x07, 0x0F, 0x10, 0x11, 0x14 ([`super::mpeg2_ext_misc`]; 0x02
//! and 0x11 are typed-opaque, their structures being defined in ISO/IEC 14496-1 /
//! 23001-13). Every other body is carried verbatim as
//! [`Mpeg2ExtensionBody::Other`] so the descriptor always round-trips. 0x10
//! (`MDL` = field + 1) and 0x13 (`SubstreamOffset` loop of `SubstreamCountMinus1`
//! entries) follow the text of §2.6.121 / §2.6.126; TSDuck 3.44 encodes both
//! differently (see `tests/h222_descriptors.rs`).
//!
//! ```text
//! Extension_descriptor() {
//!   descriptor_tag(8)=0x3F  descriptor_length(8)  extension_descriptor_tag(8)
//!   <body selected by extension_descriptor_tag>
//! }
//! ```

use super::h222_timing::{EvcTimingAndHrd, HevcTimingAndHrd, VvcTimingAndHrd};
use super::mpeg2_ext_codecs::{LcevcLinkage, LcevcVideo, MediaServiceKind};
use super::mpeg2_ext_hevc::{
    HevcHierarchyExtension, HevcOperationPoint, HevcSubregion, HevcTileSubstream,
};
use super::mpeg2_ext_misc::{
    AfExtensions, GreenExtension, JxsVideo, OdUpdate, QualityExtension, TimedMetadataExtension,
    VirtualSegmentation,
};
use super::mpegh_3d_audio::{
    MpegH3dAudio, MpegH3dAudioCommand, MpegH3dAudioConfig, MpegH3dAudioDrcLoudness,
    MpegH3dAudioMultiStream, MpegH3dAudioScene, MpegH3dAudioTextLabel, Reader,
};
use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag of the MPEG-2 `Extension_descriptor` (Table 2-45: 63).
pub const TAG: u8 = 0x3F;
const HEADER_LEN: usize = 2;
const EXTENSION_TAG_LEN: usize = 1;

/// `extension_descriptor_tag` values (Table 2-110).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
#[rustfmt::skip] // deliberate column-aligned labels
pub enum Mpeg2ExtensionTag {
    /// 0x00 — reserved.
    Reserved0,
    /// 0x01 — forbidden (PS only, Table 2-110).
    Forbidden,
    /// 0x02 — `ODUpdate_descriptor`.
    OdUpdate,
    /// 0x03 — `HEVC_timing_and_HRD_descriptor()`.
    HevcTimingAndHrd,
    /// 0x04 — `af_extensions_descriptor()`.
    AfExtensions,
    /// 0x05 — `HEVC_operation_point_descriptor()`.
    HevcOperationPoint,
    /// 0x06 — `HEVC_hierarchy_extension_descriptor()`.
    HevcHierarchyExtension,
    /// 0x07 — `Green_extension_descriptor()`.
    GreenExtension,
    /// 0x08 — `MPEG-H_3dAudio_descriptor()`.
    MpegH3dAudio,
    /// 0x09 — `MPEG-H_3dAudio_config_descriptor()`.
    MpegH3dAudioConfig,
    /// 0x0A — `MPEG-H_3dAudio_scene_descriptor()`.
    MpegH3dAudioScene,
    /// 0x0B — `MPEG-H_3dAudio_text_label_descriptor()`.
    MpegH3dAudioTextLabel,
    /// 0x0C — `MPEG-H_3dAudio_multi-stream_descriptor()`.
    MpegH3dAudioMultiStream,
    /// 0x0D — `MPEG-H_3dAudio_drc_loudness_descriptor()`.
    MpegH3dAudioDrcLoudness,
    /// 0x0E — `MPEG-H_3dAudio_command_descriptor()`.
    MpegH3dAudioCommand,
    /// 0x0F — `Quality_extension_descriptor()`.
    QualityExtension,
    /// 0x10 — `Virtual_segmentation_descriptor()`.
    VirtualSegmentation,
    /// 0x11 — `timed_metadata_extension_descriptor()`.
    TimedMetadataExtension,
    /// 0x12 — `HEVC_tile_substream_descriptor()`.
    HevcTileSubstream,
    /// 0x13 — `HEVC_subregion_descriptor()`.
    HevcSubregion,
    /// 0x14 — `JXS_video_descriptor()`.
    JxsVideo,
    /// 0x15 — `VVC_timing_and_HRD_descriptor()`.
    VvcTimingAndHrd,
    /// 0x16 — `EVC_timing_and_HRD_descriptor()`.
    EvcTimingAndHrd,
    /// 0x17 — `LCEVC_video_descriptor()`.
    LcevcVideo,
    /// 0x18 — `LCEVC_linkage_descriptor()`.
    LcevcLinkage,
    /// 0x19 — `Media_service_kind_descriptor()`.
    MediaServiceKind,
    /// 0x1A..=0xFF — reserved by ITU-T H.222.0 | ISO/IEC 13818-1; carries the value.
    Reserved(u8),
}

impl Mpeg2ExtensionTag {
    /// Decode an `extension_descriptor_tag` byte (lossless).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Reserved0,
            0x01 => Self::Forbidden,
            0x02 => Self::OdUpdate,
            0x03 => Self::HevcTimingAndHrd,
            0x04 => Self::AfExtensions,
            0x05 => Self::HevcOperationPoint,
            0x06 => Self::HevcHierarchyExtension,
            0x07 => Self::GreenExtension,
            0x08 => Self::MpegH3dAudio,
            0x09 => Self::MpegH3dAudioConfig,
            0x0A => Self::MpegH3dAudioScene,
            0x0B => Self::MpegH3dAudioTextLabel,
            0x0C => Self::MpegH3dAudioMultiStream,
            0x0D => Self::MpegH3dAudioDrcLoudness,
            0x0E => Self::MpegH3dAudioCommand,
            0x0F => Self::QualityExtension,
            0x10 => Self::VirtualSegmentation,
            0x11 => Self::TimedMetadataExtension,
            0x12 => Self::HevcTileSubstream,
            0x13 => Self::HevcSubregion,
            0x14 => Self::JxsVideo,
            0x15 => Self::VvcTimingAndHrd,
            0x16 => Self::EvcTimingAndHrd,
            0x17 => Self::LcevcVideo,
            0x18 => Self::LcevcLinkage,
            0x19 => Self::MediaServiceKind,
            other => Self::Reserved(other),
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Reserved0 => 0x00,
            Self::Forbidden => 0x01,
            Self::OdUpdate => 0x02,
            Self::HevcTimingAndHrd => 0x03,
            Self::AfExtensions => 0x04,
            Self::HevcOperationPoint => 0x05,
            Self::HevcHierarchyExtension => 0x06,
            Self::GreenExtension => 0x07,
            Self::MpegH3dAudio => 0x08,
            Self::MpegH3dAudioConfig => 0x09,
            Self::MpegH3dAudioScene => 0x0A,
            Self::MpegH3dAudioTextLabel => 0x0B,
            Self::MpegH3dAudioMultiStream => 0x0C,
            Self::MpegH3dAudioDrcLoudness => 0x0D,
            Self::MpegH3dAudioCommand => 0x0E,
            Self::QualityExtension => 0x0F,
            Self::VirtualSegmentation => 0x10,
            Self::TimedMetadataExtension => 0x11,
            Self::HevcTileSubstream => 0x12,
            Self::HevcSubregion => 0x13,
            Self::JxsVideo => 0x14,
            Self::VvcTimingAndHrd => 0x15,
            Self::EvcTimingAndHrd => 0x16,
            Self::LcevcVideo => 0x17,
            Self::LcevcLinkage => 0x18,
            Self::MediaServiceKind => 0x19,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (Table 2-110).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved0 => "reserved",
            Self::Forbidden => "forbidden",
            Self::OdUpdate => "ODUpdate_descriptor",
            Self::HevcTimingAndHrd => "HEVC_timing_and_HRD_descriptor",
            Self::AfExtensions => "af_extensions_descriptor",
            Self::HevcOperationPoint => "HEVC_operation_point_descriptor",
            Self::HevcHierarchyExtension => "HEVC_hierarchy_extension_descriptor",
            Self::GreenExtension => "Green_extension_descriptor",
            Self::MpegH3dAudio => "MPEG-H_3dAudio_descriptor",
            Self::MpegH3dAudioConfig => "MPEG-H_3dAudio_config_descriptor",
            Self::MpegH3dAudioScene => "MPEG-H_3dAudio_scene_descriptor",
            Self::MpegH3dAudioTextLabel => "MPEG-H_3dAudio_text_label_descriptor",
            Self::MpegH3dAudioMultiStream => "MPEG-H_3dAudio_multi-stream_descriptor",
            Self::MpegH3dAudioDrcLoudness => "MPEG-H_3dAudio_drc_loudness_descriptor",
            Self::MpegH3dAudioCommand => "MPEG-H_3dAudio_command_descriptor",
            Self::QualityExtension => "Quality_extension_descriptor",
            Self::VirtualSegmentation => "Virtual_segmentation_descriptor",
            Self::TimedMetadataExtension => "timed_metadata_extension_descriptor",
            Self::HevcTileSubstream => "HEVC_tile_substream_descriptor",
            Self::HevcSubregion => "HEVC_subregion_descriptor",
            Self::JxsVideo => "JXS_video_descriptor",
            Self::VvcTimingAndHrd => "VVC_timing_and_HRD_descriptor",
            Self::EvcTimingAndHrd => "EVC_timing_and_HRD_descriptor",
            Self::LcevcVideo => "LCEVC_video_descriptor",
            Self::LcevcLinkage => "LCEVC_linkage_descriptor",
            Self::MediaServiceKind => "Media_service_kind_descriptor",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(Mpeg2ExtensionTag, Reserved);

/// Body of an MPEG-2 `Extension_descriptor`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Mpeg2ExtensionBody {
    /// `extension_descriptor_tag` 0x08 (Table 2-121).
    MpegH3dAudio(MpegH3dAudio),
    /// 0x09 (Table 2-122).
    MpegH3dAudioConfig(MpegH3dAudioConfig),
    /// 0x0A (Table 2-123).
    MpegH3dAudioScene(MpegH3dAudioScene),
    /// 0x0B (Table 2-124).
    MpegH3dAudioTextLabel(MpegH3dAudioTextLabel),
    /// 0x0C (Table 2-125).
    MpegH3dAudioMultiStream(MpegH3dAudioMultiStream),
    /// 0x0D (Table 2-126).
    MpegH3dAudioDrcLoudness(MpegH3dAudioDrcLoudness),
    /// 0x0E (Table 2-127).
    MpegH3dAudioCommand(MpegH3dAudioCommand),
    /// `extension_descriptor_tag` 0x02 (Table 2-109 `ObjectDescriptorUpdate()`, opaque).
    OdUpdate(OdUpdate),
    /// `extension_descriptor_tag` 0x03 (Table 2-115).
    HevcTimingAndHrd(HevcTimingAndHrd),
    /// `extension_descriptor_tag` 0x04 (Table 2-116).
    AfExtensions(AfExtensions),
    /// `extension_descriptor_tag` 0x05 (Table 2-117).
    HevcOperationPoint(HevcOperationPoint),
    /// `extension_descriptor_tag` 0x06 (Table 2-118).
    HevcHierarchyExtension(HevcHierarchyExtension),
    /// `extension_descriptor_tag` 0x07 (Table 2-120).
    GreenExtension(GreenExtension),
    /// `extension_descriptor_tag` 0x0F (Table 2-128).
    QualityExtension(QualityExtension),
    /// `extension_descriptor_tag` 0x10 (Table 2-129).
    VirtualSegmentation(VirtualSegmentation),
    /// `extension_descriptor_tag` 0x11 (ISO/IEC 23001-13, opaque).
    TimedMetadataExtension(TimedMetadataExtension),
    /// `extension_descriptor_tag` 0x12 (Table 2-130).
    HevcTileSubstream(HevcTileSubstream),
    /// `extension_descriptor_tag` 0x13 (Table 2-131).
    HevcSubregion(HevcSubregion),
    /// `extension_descriptor_tag` 0x14 (Table 2-132).
    JxsVideo(JxsVideo),
    /// `extension_descriptor_tag` 0x15 (Table 2-139).
    VvcTimingAndHrd(VvcTimingAndHrd),
    /// `extension_descriptor_tag` 0x16 (Table 2-141).
    EvcTimingAndHrd(EvcTimingAndHrd),
    /// `extension_descriptor_tag` 0x17 (Table 2-142).
    LcevcVideo(LcevcVideo),
    /// `extension_descriptor_tag` 0x18 (Table 2-143).
    LcevcLinkage(LcevcLinkage),
    /// `extension_descriptor_tag` 0x19 (Table 2-144).
    MediaServiceKind(MediaServiceKind),
    /// Any other `extension_descriptor_tag`: the body bytes verbatim.
    Other {
        /// The raw `extension_descriptor_tag`.
        extension_descriptor_tag: u8,
        /// Body bytes after `extension_descriptor_tag`.
        data: Vec<u8>,
    },
}

impl Mpeg2ExtensionBody {
    /// The `extension_descriptor_tag` this body is selected by.
    #[must_use]
    pub const fn extension_descriptor_tag(&self) -> u8 {
        match self {
            Self::MpegH3dAudio(_) => 0x08,
            Self::MpegH3dAudioConfig(_) => 0x09,
            Self::MpegH3dAudioScene(_) => 0x0A,
            Self::MpegH3dAudioTextLabel(_) => 0x0B,
            Self::MpegH3dAudioMultiStream(_) => 0x0C,
            Self::MpegH3dAudioDrcLoudness(_) => 0x0D,
            Self::MpegH3dAudioCommand(_) => 0x0E,
            Self::OdUpdate(_) => 0x02,
            Self::HevcTimingAndHrd(_) => 0x03,
            Self::AfExtensions(_) => 0x04,
            Self::HevcOperationPoint(_) => 0x05,
            Self::HevcHierarchyExtension(_) => 0x06,
            Self::GreenExtension(_) => 0x07,
            Self::QualityExtension(_) => 0x0F,
            Self::VirtualSegmentation(_) => 0x10,
            Self::TimedMetadataExtension(_) => 0x11,
            Self::HevcTileSubstream(_) => 0x12,
            Self::HevcSubregion(_) => 0x13,
            Self::JxsVideo(_) => 0x14,
            Self::VvcTimingAndHrd(_) => 0x15,
            Self::EvcTimingAndHrd(_) => 0x16,
            Self::LcevcVideo(_) => 0x17,
            Self::LcevcLinkage(_) => 0x18,
            Self::MediaServiceKind(_) => 0x19,
            Self::Other {
                extension_descriptor_tag,
                ..
            } => *extension_descriptor_tag,
        }
    }

    /// Typed view of [`Self::extension_descriptor_tag`].
    #[must_use]
    pub const fn kind(&self) -> Mpeg2ExtensionTag {
        Mpeg2ExtensionTag::from_u8(self.extension_descriptor_tag())
    }

    /// Spec label of the selected descriptor (Table 2-110).
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.kind().name()
    }
}
broadcast_common::impl_spec_display!(Mpeg2ExtensionBody);

/// MPEG-2 `Extension_descriptor` (Table 2-109).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Mpeg2ExtensionDescriptor {
    /// The typed (or verbatim) body.
    pub body: Mpeg2ExtensionBody,
}

/// `extension_descriptor_tag` values that have a typed [`Mpeg2ExtensionBody`]
/// variant (so a hand-built [`Mpeg2ExtensionBody::Other`] must not use them).
const fn is_typed_extension_tag(tag: u8) -> bool {
    matches!(tag, 0x02..=0x19)
}

impl Mpeg2ExtensionDescriptor {
    /// Wrap a body.
    #[must_use]
    pub const fn new(body: Mpeg2ExtensionBody) -> Self {
        Self { body }
    }

    fn encode_body(&self) -> Result<Vec<u8>> {
        // A hand-built `Other` carrying a typed tag would serialize as that
        // typed body and parse back as a different variant (serialize -> parse
        // not an identity); the typed variants are the only way to write 0x08..=0x0E.
        if let Mpeg2ExtensionBody::Other {
            extension_descriptor_tag,
            ..
        } = &self.body
            && is_typed_extension_tag(*extension_descriptor_tag)
        {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "Mpeg2ExtensionBody::Other carries a typed extension_descriptor_tag; use the typed variant",
            });
        }
        let mut out = Vec::new();
        out.push(self.body.extension_descriptor_tag());
        match &self.body {
            Mpeg2ExtensionBody::MpegH3dAudio(b) => b.write(&mut out)?,
            Mpeg2ExtensionBody::MpegH3dAudioConfig(b) => out.extend_from_slice(&b.mpegh3da_config),
            Mpeg2ExtensionBody::MpegH3dAudioScene(b) => b.write(&mut out)?,
            Mpeg2ExtensionBody::MpegH3dAudioTextLabel(b) => b.write(&mut out)?,
            Mpeg2ExtensionBody::MpegH3dAudioMultiStream(b) => b.write(&mut out)?,
            Mpeg2ExtensionBody::MpegH3dAudioDrcLoudness(b) => b.write(&mut out)?,
            Mpeg2ExtensionBody::MpegH3dAudioCommand(b) => out.extend_from_slice(&b.mhas_packet),
            Mpeg2ExtensionBody::OdUpdate(b) => out.extend_from_slice(&b.object_descriptor_update),
            Mpeg2ExtensionBody::HevcTimingAndHrd(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::AfExtensions(b) => out.extend_from_slice(&b.trailing),
            Mpeg2ExtensionBody::HevcOperationPoint(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::HevcHierarchyExtension(b) => {
                out.extend_from_slice(&b.write_body()?)
            }
            Mpeg2ExtensionBody::GreenExtension(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::QualityExtension(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::VirtualSegmentation(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::TimedMetadataExtension(b) => out.extend_from_slice(&b.data),
            Mpeg2ExtensionBody::HevcTileSubstream(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::HevcSubregion(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::JxsVideo(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::VvcTimingAndHrd(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::EvcTimingAndHrd(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::LcevcVideo(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::LcevcLinkage(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::MediaServiceKind(b) => out.extend_from_slice(&b.write_body()?),
            Mpeg2ExtensionBody::Other { data, .. } => out.extend_from_slice(data),
        }
        Ok(out)
    }
}

impl<'a> Parse<'a> for Mpeg2ExtensionDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Mpeg2ExtensionDescriptor",
            "unexpected tag for Extension_descriptor",
        )?;
        let (&ext, rest) = body.split_first().ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "Extension_descriptor needs extension_descriptor_tag",
        })?;
        let mut r = Reader::new(rest, "Mpeg2ExtensionDescriptor");
        let body = match ext {
            0x08 => Mpeg2ExtensionBody::MpegH3dAudio(MpegH3dAudio::parse(&mut r)?),
            0x09 => Mpeg2ExtensionBody::MpegH3dAudioConfig(MpegH3dAudioConfig {
                mpegh3da_config: rest.to_vec(),
            }),
            0x0A => Mpeg2ExtensionBody::MpegH3dAudioScene(MpegH3dAudioScene::parse(&mut r)?),
            0x0B => {
                Mpeg2ExtensionBody::MpegH3dAudioTextLabel(MpegH3dAudioTextLabel::parse(&mut r)?)
            }
            0x0C => {
                Mpeg2ExtensionBody::MpegH3dAudioMultiStream(MpegH3dAudioMultiStream::parse(&mut r)?)
            }
            0x0D => {
                Mpeg2ExtensionBody::MpegH3dAudioDrcLoudness(MpegH3dAudioDrcLoudness::parse(&mut r)?)
            }
            0x0E => Mpeg2ExtensionBody::MpegH3dAudioCommand(MpegH3dAudioCommand {
                mhas_packet: rest.to_vec(),
            }),
            0x02 => Mpeg2ExtensionBody::OdUpdate(OdUpdate {
                object_descriptor_update: rest.to_vec(),
            }),
            0x03 => Mpeg2ExtensionBody::HevcTimingAndHrd(HevcTimingAndHrd::parse_body(rest)?),
            0x04 => Mpeg2ExtensionBody::AfExtensions(AfExtensions {
                trailing: rest.to_vec(),
            }),
            0x05 => Mpeg2ExtensionBody::HevcOperationPoint(HevcOperationPoint::parse_body(rest)?),
            0x06 => Mpeg2ExtensionBody::HevcHierarchyExtension(HevcHierarchyExtension::parse_body(
                rest,
            )?),
            0x07 => Mpeg2ExtensionBody::GreenExtension(GreenExtension::parse_body(rest)?),
            0x0F => Mpeg2ExtensionBody::QualityExtension(QualityExtension::parse_body(rest)?),
            0x10 => Mpeg2ExtensionBody::VirtualSegmentation(VirtualSegmentation::parse_body(rest)?),
            0x11 => Mpeg2ExtensionBody::TimedMetadataExtension(TimedMetadataExtension {
                data: rest.to_vec(),
            }),
            0x12 => Mpeg2ExtensionBody::HevcTileSubstream(HevcTileSubstream::parse_body(rest)?),
            0x13 => Mpeg2ExtensionBody::HevcSubregion(HevcSubregion::parse_body(rest)?),
            0x14 => Mpeg2ExtensionBody::JxsVideo(JxsVideo::parse_body(rest)?),
            0x15 => Mpeg2ExtensionBody::VvcTimingAndHrd(VvcTimingAndHrd::parse_body(rest)?),
            0x16 => Mpeg2ExtensionBody::EvcTimingAndHrd(EvcTimingAndHrd::parse_body(rest)?),
            0x17 => Mpeg2ExtensionBody::LcevcVideo(LcevcVideo::parse_body(rest)?),
            0x18 => Mpeg2ExtensionBody::LcevcLinkage(LcevcLinkage::parse_body(rest)?),
            0x19 => Mpeg2ExtensionBody::MediaServiceKind(MediaServiceKind::parse_body(rest)?),
            other => Mpeg2ExtensionBody::Other {
                extension_descriptor_tag: other,
                data: rest.to_vec(),
            },
        };
        Ok(Self { body })
    }
}

impl Serialize for Mpeg2ExtensionDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        // An unencodable value is reported by `serialize_into`; 0 here keeps
        // the length query infallible.
        self.encode_body()
            .map_or(0, |b| HEADER_LEN + b.len().max(EXTENSION_TAG_LEN))
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body = self.encode_body()?;
        let len = HEADER_LEN + body.len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, body.len())?;
        buf[HEADER_LEN..len].copy_from_slice(&body);
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for Mpeg2ExtensionDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MPEG2_EXTENSION";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::mpegh_3d_audio::*;

    fn d(body: &[u8]) -> Vec<u8> {
        let mut v = vec![TAG, body.len() as u8];
        v.extend_from_slice(body);
        v
    }

    fn rt(bytes: &[u8]) -> Mpeg2ExtensionDescriptor {
        let p = Mpeg2ExtensionDescriptor::parse(bytes).unwrap();
        let mut out = vec![0u8; p.serialized_len()];
        let n = p.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "serialize must be byte-identical");
        assert_eq!(Mpeg2ExtensionDescriptor::parse(&out[..n]).unwrap(), p);
        p
    }

    fn body(p: &Mpeg2ExtensionDescriptor) -> &Mpeg2ExtensionBody {
        &p.body
    }

    // Real PMT ES_info bytes of the Fraunhofer-IIS mpegh-test-content transport
    // streams (PMT of `mpegh_cicp_01_baseline_32kbps_single.ts` etc.): the
    // MPEG-H 3D audio descriptor with all-ones reserved bits.
    #[test]
    fn fraunhofer_baseline_cicp01() {
        let p = rt(&[0x3F, 0x04, 0x08, 0x10, 0x7F, 0xC1]);
        let Mpeg2ExtensionBody::MpegH3dAudio(a) = body(&p) else {
            panic!()
        };
        assert_eq!(a.profile_level_indication, 0x10);
        assert!(!a.interactivity_enabled);
        assert_eq!(a.compatible_sets, None);
        assert_eq!(a.reference_channel_layout, 1);
        assert_eq!((a.reserved_flags, a.reserved_layout), (0x3F, 0x3));
        assert!(a.trailing.is_empty());
    }

    #[test]
    fn fraunhofer_lc_with_compatible_set() {
        let p = rt(&[0x3F, 0x06, 0x08, 0x0B, 0x3F, 0xC1, 0x01, 0x10]);
        let Mpeg2ExtensionBody::MpegH3dAudio(a) = body(&p) else {
            panic!()
        };
        assert_eq!(a.profile_level_indication, 0x0B);
        assert_eq!(a.compatible_sets.as_deref(), Some(&[0x10][..]));
        assert_eq!(a.reference_channel_layout, 1);
    }

    #[test]
    fn fraunhofer_cicp13_and_06() {
        let p = rt(&[0x3F, 0x04, 0x08, 0x13, 0x7F, 0xCD]);
        let Mpeg2ExtensionBody::MpegH3dAudio(a) = body(&p) else {
            panic!()
        };
        assert_eq!(a.reference_channel_layout, 13);
        rt(&[0x3F, 0x06, 0x08, 0x0C, 0x3F, 0xC6, 0x01, 0x11]);
    }

    // TSDuck (secondary cross-check) `tstabcomp` binary for the same syntax,
    // including reserved trailing bytes and a non-default interactivity flag.
    #[test]
    fn tsduck_3d_audio_vectors() {
        let p = rt(&[
            0x3F, 0x09, 0x08, 0xAB, 0x7F, 0xF4, 0x01, 0x23, 0x45, 0x67, 0x89,
        ]);
        let Mpeg2ExtensionBody::MpegH3dAudio(a) = body(&p) else {
            panic!()
        };
        assert_eq!(a.reference_channel_layout, 0x34);
        assert_eq!(a.trailing, [0x01, 0x23, 0x45, 0x67, 0x89]);
        let p = rt(&[0x3F, 0x04, 0x08, 0x01, 0xFF, 0xC2]);
        let Mpeg2ExtensionBody::MpegH3dAudio(a) = body(&p) else {
            panic!()
        };
        assert!(a.interactivity_enabled);
        assert_eq!(a.reference_channel_layout, 2);
    }

    #[test]
    fn tsduck_multi_stream_vectors() {
        let p = rt(&[0x3F, 0x05, 0x0C, 0x71, 0x01, 0x23, 0x45]);
        let Mpeg2ExtensionBody::MpegH3dAudioMultiStream(m) = body(&p) else {
            panic!()
        };
        assert!(m.main.is_none());
        assert_eq!(m.this_stream_id, 0x71);
        assert_eq!(m.trailing, [0x01, 0x23, 0x45]);

        let p = rt(&[
            0x3F, 0x09, 0x0C, 0x8F, 0x8B, 0x83, 0x25, 0x26, 0xDA, 0x28, 0x5B,
        ]);
        let Mpeg2ExtensionBody::MpegH3dAudioMultiStream(m) = body(&p) else {
            panic!()
        };
        let main = m.main.as_ref().unwrap();
        assert_eq!(m.this_stream_id, 0x0F);
        assert_eq!(main.num_auxiliary_streams, 11);
        assert_eq!(main.groups.len(), 3);
        assert_eq!(main.groups[0].group_id, 0x12);
        assert!(main.groups[0].is_in_main_stream && main.groups[0].auxiliary.is_none());
        let a1 = main.groups[1].auxiliary.unwrap();
        assert_eq!(
            (main.groups[1].group_id, a1.is_in_ts, a1.auxiliary_stream_id),
            (0x13, true, 0x5A)
        );
        let a2 = main.groups[2].auxiliary.unwrap();
        assert_eq!(
            (main.groups[2].group_id, a2.is_in_ts, a2.auxiliary_stream_id),
            (0x14, false, 0x5B)
        );
    }

    // Vectors hand-derived from Table 2-123 (no real capture exists locally).
    #[test]
    fn scene_vector_from_table_2_123() {
        let b = [
            0x0A, 0xFF, 0x07, 0x81, 0x85, 0xF9, 0xF3, b'e', b'n', b'g', 0xE1, 0x8A, 0xE1, 0x8A,
            0x8B, 0x8A, 0xE1, 0xE1, 0xE2, 0xF0, 0x0B, 0xF6, 0x20,
        ];
        let p = rt(&d(&b));
        let Mpeg2ExtensionBody::MpegH3dAudioScene(s) = body(&p) else {
            panic!()
        };
        assert_eq!(s.scene_info_id, 7);
        let g = &s.groups.as_ref().unwrap()[0];
        assert_eq!(
            (g.group_id, g.allow_on_off, g.default_on_off, g.content_kind),
            (5, true, true, 3)
        );
        assert_eq!(g.content_language, Some(*b"eng"));
        assert!(g.position.is_none() && g.gain.is_none());
        let sg = &s.switch_groups.as_ref().unwrap()[0];
        assert_eq!(
            (sg.switch_group_id, sg.allow_on_off, sg.default_on_off),
            (2, true, false)
        );
        assert_eq!(
            sg.members.iter().map(|m| m.member_id).collect::<Vec<_>>(),
            [10, 11]
        );
        assert_eq!(sg.default_group_id, 10);
        let pr = &s.presets.as_ref().unwrap()[0];
        assert_eq!((pr.preset_id, pr.preset_kind), (1, 2));
        let det = pr.conditions[0].detail.as_ref().unwrap();
        assert_eq!(pr.conditions[0].group_id, 5);
        assert!(!det.disable_gain_interactivity && det.disable_position_interactivity);
        assert_eq!((det.gain, det.position), (Some(0x20), None));
    }

    #[test]
    fn scene_with_interactivity_blocks_round_trips() {
        let mut g = SceneGroup::new(1, true, false, 2);
        g.position = Some(PositionInteractivity::new(1, 2, 3, 4, 5, 6));
        g.gain = Some(GainInteractivity::new(7, 8));
        let mut pc = PresetConditionDetail {
            reserved: 15,
            disable_gain_interactivity: true,
            disable_position_interactivity: false,
            gain: Some(9),
            position: Some(PresetPosition::new(1, 2, 3)),
        };
        pc.gain = Some(9);
        let preset = GroupPreset {
            reserved_id: 7,
            preset_id: 3,
            reserved_kind: 7,
            preset_kind: 1,
            reserved_conditions: 15,
            conditions: vec![PresetCondition {
                group_id: 1,
                detail: Some(pc),
            }],
        };
        let s = MpegH3dAudioScene::new(0, Some(vec![g]), None, Some(vec![preset]));
        let desc = Mpeg2ExtensionDescriptor::new(Mpeg2ExtensionBody::MpegH3dAudioScene(s));
        let mut out = vec![0u8; desc.serialized_len()];
        let n = desc.serialize_into(&mut out).unwrap();
        assert_eq!(Mpeg2ExtensionDescriptor::parse(&out[..n]).unwrap(), desc);
    }

    #[test]
    fn text_label_vector_from_table_2_124() {
        let b = [
            0x0B, 0x03, 0xF1, b'e', b'n', b'g', 0x81, 0x85, 0x02, b'H', b'i', 0xE0, 0xE1, 0xE3,
            0x01, b'X',
        ];
        let p = rt(&d(&b));
        let Mpeg2ExtensionBody::MpegH3dAudioTextLabel(t) = body(&p) else {
            panic!()
        };
        assert_eq!(t.scene_info_id, 3);
        let l = &t.languages[0];
        assert_eq!(l.language, *b"eng");
        assert_eq!(
            (l.groups[0].id, l.groups[0].data.as_slice()),
            (5, &b"Hi"[..])
        );
        assert!(l.switch_groups.is_empty());
        assert_eq!(
            (l.presets[0].id, l.presets[0].data.as_slice()),
            (3, &b"X"[..])
        );
    }

    #[test]
    fn drc_loudness_vector_from_table_2_126() {
        let b = [
            0x0D, 0xFF, 0xC1, 0xC1, 0xE1, 0xFE, 0x89, 0xC4, 0x81, 0xE7, 0x82, 0x01, 0x02, 0x40,
            0xA1, 0xC5, 0x81, 0xFF, 0xE2, 0x02, 0xAA, 0xBB, 0x81, 0x45,
        ];
        let p = rt(&d(&b));
        let Mpeg2ExtensionBody::MpegH3dAudioDrcLoudness(x) = body(&p) else {
            panic!()
        };
        let i = x.info.as_ref().unwrap();
        let dr = &i.drc_instructions[0];
        assert!(matches!(
            dr.scope,
            DrcScope::Group {
                kind: 2,
                group_id: 9,
                ..
            }
        ));
        assert_eq!(
            (dr.drc_set_id, dr.downmix_id, dr.drc_set_effect),
            (4, 1, 0x0102)
        );
        assert_eq!(dr.additional_downmix_ids, [(1, 2)]);
        assert_eq!(dr.limiter_peak_target, Some(0x40));
        let t = dr.target_loudness.unwrap();
        assert_eq!(
            (
                t.upper,
                t.lower,
                t.depends_on_drc_set,
                t.no_independent_use_or_reserved
            ),
            (0x10, Some((3, 5)), 0, 1)
        );
        let l = &i.loudness_infos[0];
        assert!(matches!(l.scope, DrcScope::Preset { preset_id: 2, .. }));
        assert_eq!(l.loudness_info, [0xAA, 0xBB]);
        let m = i.downmix_definitions[0];
        assert_eq!(
            (m.downmix_id, m.downmix_type, m.cicp_speaker_layout_idx),
            (1, 1, 5)
        );
        // Absent payload: reserved(7)=0x7F, flag 0.
        rt(&d(&[0x0D, 0xFE]));
    }

    #[test]
    fn config_command_and_other_are_opaque() {
        let p = rt(&d(&[0x09, 1, 2, 3]));
        assert!(
            matches!(body(&p), Mpeg2ExtensionBody::MpegH3dAudioConfig(c) if c.mpegh3da_config == [1, 2, 3])
        );
        let p = rt(&d(&[0x0E, 0xAA]));
        assert!(
            matches!(body(&p), Mpeg2ExtensionBody::MpegH3dAudioCommand(c) if c.mhas_packet == [0xAA])
        );
        let p = rt(&d(&[0x1F, 9, 9]));
        assert_eq!(body(&p).kind(), Mpeg2ExtensionTag::Reserved(0x1F));
        assert_eq!(body(&p).extension_descriptor_tag(), 0x1F);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Mpeg2ExtensionDescriptor::parse(&[0x3E, 0]).is_err());
        assert!(Mpeg2ExtensionDescriptor::parse(&[TAG, 0]).is_err());
        // Group claims an auxiliary location but is in the main stream.
        let m = MpegH3dAudioMultiStream {
            this_stream_id: 0,
            main: Some(MultiStreamMain {
                reserved_aux: 1,
                num_auxiliary_streams: 0,
                reserved_groups: 1,
                groups: vec![MultiStreamGroup {
                    group_id: 1,
                    is_in_main_stream: true,
                    auxiliary: Some(AuxiliaryLocation {
                        is_in_ts: true,
                        auxiliary_stream_id: 1,
                    }),
                }],
            }),
            trailing: Vec::new(),
        };
        let desc = Mpeg2ExtensionDescriptor::new(Mpeg2ExtensionBody::MpegH3dAudioMultiStream(m));
        let mut out = [0u8; 32];
        assert!(desc.serialize_into(&mut out).is_err());
        // Over-wide reserved/field values must be rejected, not masked.
        let mut a = MpegH3dAudio::new(0, false, None, 1);
        a.reference_channel_layout = 0x40;
        let desc = Mpeg2ExtensionDescriptor::new(Mpeg2ExtensionBody::MpegH3dAudio(a));
        assert!(desc.serialize_into(&mut out).is_err());
        // A 256-byte body does not fit descriptor_length.
        let big = Mpeg2ExtensionDescriptor::new(Mpeg2ExtensionBody::MpegH3dAudioConfig(
            MpegH3dAudioConfig {
                mpegh3da_config: vec![0; 255],
            },
        ));
        assert!(big.serialize_into(&mut [0u8; 300]).is_err());
    }

    /// No truncation of any vector may panic (it must parse or return `Err`).
    #[test]
    fn truncated_bodies_never_panic() {
        let vectors: [&[u8]; 4] = [
            &[
                0x0A, 0xFF, 0x07, 0x81, 0x85, 0xF9, 0xF3, b'e', b'n', b'g', 0xE1, 0x8A, 0xE1, 0x8A,
                0x8B, 0x8A, 0xE1, 0xE1, 0xE2, 0xF0, 0x0B, 0xF6, 0x20,
            ],
            &[
                0x0B, 0x03, 0xF1, b'e', b'n', b'g', 0x81, 0x85, 0x02, b'H', b'i', 0xE0, 0xE1, 0xE3,
                0x01, b'X',
            ],
            &[
                0x0D, 0xFF, 0xC1, 0xC1, 0xE1, 0xFE, 0x89, 0xC4, 0x81, 0xE7, 0x82, 0x01, 0x02, 0x40,
                0xA1, 0xC5, 0x81, 0xFF, 0xE2, 0x02, 0xAA, 0xBB, 0x81, 0x45,
            ],
            &[0x0C, 0x8F, 0x8B, 0x83, 0x25, 0x26, 0xDA, 0x28, 0x5B],
        ];
        for v in vectors {
            for cut in 0..v.len() {
                let _ = Mpeg2ExtensionDescriptor::parse(&d(&v[..cut]));
            }
        }
    }

    #[test]
    fn tag_enum_round_trips_all_256() {
        for v in 0..=255u8 {
            assert_eq!(Mpeg2ExtensionTag::from_u8(v).to_u8(), v);
        }
        assert_eq!(
            Mpeg2ExtensionTag::from_u8(8).name(),
            "MPEG-H_3dAudio_descriptor"
        );
    }
}
