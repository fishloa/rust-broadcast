//! Display geometry of a video track: what a player needs to show each
//! picture at its correct shape.
//!
//! Phases 1a and 1b of `docs/display-geometry/DESIGN.md`: the **sample aspect
//! ratio** (SAR) signalled in the SPS VUI, the coded picture size, and the
//! **video signal type / colour description** (video format, full range and
//! the colour primaries / transfer characteristics / matrix coefficients),
//! derived once per track into [`TrackSpec::display`](super::TrackSpec::display).
//! **Phase 2** adds the change model: [`DisplayInfo`] (the displayed state of
//! one access unit), [`DisplayChangeSet`] (which parts of it changed) and
//! [`DisplayInfo::changes_from`], which compares parsed geometry by value,
//! never by the parameter-set bytes it arrived in. Cropping is a later phase
//! (every type here is `#[non_exhaustive]`, so it lands additively).
//!
//! # Spec citations
//!
//! - **H.264**: ITU-T H.264 (V15, 08/2024) §E.1.1 (`vui_parameters()` syntax),
//!   §E.2.1 Table E-1 (meaning of `aspect_ratio_idc`), Table E-2
//!   (`video_format`), Table E-3 (`colour_primaries`), Table E-4
//!   (`transfer_characteristics`) and Table E-5 (`matrix_coefficients`).
//! - **H.265**: ITU-T H.265 (V11, 01/2026) §E.2.1 (syntax) and §E.3.1
//!   Tables E.1 (same values as Table E-1), E.2, E.3, E.4 and E.5.
//!
//! Every table was re-verified row by row against the rendered PDF pages and
//! is transcribed in `private/specs-verified/mpeg/transmux/codec/vui-colour-tables.md` (see also
//! `private/specs-verified/mpeg/display-geometry/public-notes/SPEC-NOTES.md`), not read from the markdown text,
//! whose Table E-1 rows are merged and whose Tables E-3..E-5 lost their
//! column structure.
//!
//! # Semantics kept exactly
//!
//! - `aspect_ratio_idc` absent (no VUI, or `aspect_ratio_info_present_flag`
//!   0) is inferred 0 (unspecified): [`DisplayGeometry::aspect_ratio`] is
//!   `None`.
//! - "When `aspect_ratio_idc` is equal to 0 or `sar_width` is equal to 0 or
//!   `sar_height` is equal to 0, the sample aspect ratio shall be considered
//!   unspecified": [`AspectRatioIdc::sample_aspect_ratio`] is `None`, never a
//!   `0:0` sentinel.
//! - H.265 §E.3.1: values 17..=254 are reserved and "decoders shall interpret"
//!   them "as equivalent to the value 0". H.264 marks the same range reserved
//!   without that sentence; this crate applies the H.265 rule to both.
//! - Two broadcasters code a 720x576 16:9 picture as `16:11` (idc 4, Table
//!   E-1 NOTE 1: horizontal overscan) and as `64:45` (Extended_SAR). Both are
//!   legitimate and are kept exactly as signalled, never normalised to each
//!   other.

use crate::error::Result;
use crate::ir::CodecConfig;
use broadcast_bitstream::facts::{
    ActiveFormatDescription, AspectRatioIdc, BarData, ColourDescription, PictureRect,
    SampleAspectRatio, VideoFormat, VideoSignalType,
};
use broadcast_bitstream::sps::SpsDisplayFacts;

/// The spec's inferred `video_format` when `video_signal_type_present_flag`
/// is 0 (H.264 §E.2.1: "shall be inferred to be equal to 5").
pub const INFERRED_VIDEO_FORMAT: u8 = 5;

/// The spec's inferred `video_full_range_flag` when absent (H.264 §E.2.1: "= 0").
pub const INFERRED_FULL_RANGE: bool = false;

/// The spec's inferred `colour_primaries` / `transfer_characteristics` /
/// `matrix_coefficients` when absent (H.264 §E.2.1: "= 2", unspecified).
pub const INFERRED_COLOUR_CODE: u8 = 2;

/// The MPEG-2 inferred `colour_primaries` / `transfer_characteristics` /
/// `matrix_coefficients` when `sequence_display_extension()` or its
/// `colour_description` is absent: value 1 in all three, i.e. BT.709
/// (ITU-T H.262 §6.3.6, printed pp. 44/47; `private/specs-verified/mpeg/transmux/codec/mpeg2-video-display.md`).
/// Distinct from [`INFERRED_COLOUR_CODE`]: MPEG-2 never infers "unspecified".
pub const MPEG2_INFERRED_COLOUR_CODE: u8 = 1;

/// Sequence-level display geometry of a video track: everything derived from
/// the active SPS (in a conforming stream it changes only at an IDR / IRAP,
/// H.264 §7.4.1.2.1 / H.265 §7.4.3.2). Lives on
/// [`TrackSpec::display`](super::TrackSpec::display).
///
/// Phases 1a/1b carry the coded size, the VUI aspect ratio, the overscan
/// flag and the video signal type (format, full range, colour description);
/// cropping and the H.265 default display window are additive follow-ups.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayGeometry {
    /// Decoded picture width before cropping, in luma samples (H.264
    /// `PicWidthInSamplesL`; H.265 `pic_width_in_luma_samples`). `u32`, wider
    /// than the `u16` [`CodecConfig`] carries; unlike `CodecConfig`'s width it
    /// is **not** cropped.
    pub coded_width: u32,
    /// Decoded picture height before cropping, in luma samples (H.264
    /// `16 * FrameHeightInMbs`; H.265 `pic_height_in_luma_samples`).
    pub coded_height: u32,
    /// What the decoder outputs: H.264 frame-cropping rectangle / H.265
    /// conformance window. Equals the full picture when there is no cropping.
    pub output_rect: PictureRect,
    /// H.265 default display window (absolute, i.e. conformance offsets
    /// already added per E-78..E-81). Always `None` for H.264. A suggestion,
    /// never applied to `output_rect`.
    pub default_display_rect: Option<PictureRect>,
    /// The VUI `aspect_ratio_idc` as signalled. `None` when the SPS has no VUI,
    /// `aspect_ratio_info_present_flag` is 0, or the VUI was truncated before
    /// the field was complete (all inferred idc 0, "unspecified"). `Some` does
    /// **not** imply a specified ratio: see
    /// [`sample_aspect_ratio`](Self::sample_aspect_ratio).
    pub aspect_ratio: Option<AspectRatioIdc>,
    /// `overscan_appropriate_flag` as signalled. `None` when
    /// `overscan_info_present_flag` is 0 or the VUI was truncated before it.
    pub overscan_appropriate: Option<bool>,
    /// `video_signal_type()` as signalled. `None` when
    /// `video_signal_type_present_flag` is 0 (or there is no VUI). The spec's
    /// inferred defaults (format 5, limited range, colour 2/2/2) are then
    /// available through [`video_format`](Self::video_format),
    /// [`full_range`](Self::full_range) and
    /// [`colour_description`](Self::colour_description); the struct itself
    /// records only what was signalled.
    pub video_signal: Option<VideoSignalType>,
    /// This geometry came from MPEG-2 video (H.262) syntax, whose inferred
    /// colour defaults differ from H.264/H.265: BT.709 (1/1/1) rather than
    /// unspecified (2/2/2, H.262 §6.3.6 vs H.264 §E.2.1). `false` for every
    /// other source; [`colour_description`](Self::colour_description) reads it
    /// when nothing was signalled.
    pub mpeg2: bool,
}

impl DisplayGeometry {
    /// Build from explicit values (for muxers and tests; demuxed tracks get
    /// theirs from the SPS).
    pub fn new(coded_width: u32, coded_height: u32, aspect_ratio: Option<AspectRatioIdc>) -> Self {
        Self {
            coded_width,
            coded_height,
            output_rect: PictureRect::new(0, 0, coded_width, coded_height),
            default_display_rect: None,
            aspect_ratio,
            overscan_appropriate: None,
            video_signal: None,
            mpeg2: false,
        }
    }

    /// Geometry of an H.264 SPS NAL unit (1-byte NAL header included; the same
    /// input as `broadcast_bitstream::sps::decode_avc_sps`, and the same VUI
    /// walk).
    pub fn from_avc_sps(sps_nal: &[u8]) -> Result<Self> {
        broadcast_bitstream::sps::decode_avc_display_facts(sps_nal)
            .map(Self::from_facts)
            .map_err(Into::into)
    }

    /// Geometry of an H.265 SPS NAL unit (2-byte NAL header included; the same
    /// input as `broadcast_bitstream::sps::decode_hevc_sps`).
    pub fn from_hevc_sps(sps_nal: &[u8]) -> Result<Self> {
        broadcast_bitstream::sps::decode_hevc_display_facts(sps_nal)
            .map(Self::from_facts)
            .map_err(Into::into)
    }

    /// Geometry derived from the first SPS of an `avcC` / `hvcC` record, or
    /// from the `sequence_header()` an MPEG-2 video `esds`
    /// DecoderSpecificInfo carries. `None` for other codecs, for a record with
    /// no SPS / no sequence header (a placeholder config), or when that header
    /// does not decode; never a fabricated value.
    pub fn from_codec_config(config: &CodecConfig) -> Option<Self> {
        match config {
            CodecConfig::Avc { config, .. } => {
                let sps = config.config.sps.first()?;
                Self::from_avc_sps(&sps.0).ok()
            }
            CodecConfig::Hevc { config, .. } => {
                let sps = config
                    .config
                    .arrays
                    .iter()
                    .filter(|a| a.nal_unit_type == isobmff::nalu_types::HEVC_SPS_NUT)
                    .flat_map(|a| a.nalus.iter())
                    .next()?;
                Self::from_hevc_sps(&sps.0).ok()
            }
            CodecConfig::Mpeg2Video { esds, .. } => {
                let es = esds
                    .es_descriptor
                    .decoder_config
                    .as_ref()?
                    .decoder_specific_info
                    .as_ref()?;
                Self::from_mpeg2_es(&es.data).ok()
            }
            _ => None,
        }
    }

    fn from_facts(f: SpsDisplayFacts) -> Self {
        Self {
            coded_width: f.coded_width(),
            coded_height: f.coded_height(),
            output_rect: f.output_rect(),
            default_display_rect: f.default_display_rect(),
            aspect_ratio: f.aspect_ratio(),
            overscan_appropriate: f.overscan_appropriate(),
            video_signal: f.video_signal(),
            mpeg2: false,
        }
    }

    /// Whether this geometry came from an MPEG-2 (H.262) sequence.
    pub fn is_mpeg2(&self) -> bool {
        self.mpeg2
    }

    /// The specified sample aspect ratio as `(width, height)`, or `None` when
    /// it is unspecified (no VUI / flag clear, idc 0, reserved idc, or an
    /// Extended_SAR with a zero component). Exactly as signalled, not reduced:
    /// see [`SampleAspectRatio::reduced`].
    pub fn sample_aspect_ratio(&self) -> Option<(u16, u16)> {
        self.typed_sample_aspect_ratio().map(Into::into)
    }

    /// [`sample_aspect_ratio`](Self::sample_aspect_ratio) as the typed
    /// [`SampleAspectRatio`].
    pub fn typed_sample_aspect_ratio(&self) -> Option<SampleAspectRatio> {
        self.aspect_ratio?.sample_aspect_ratio()
    }

    /// `video_full_range_flag`: `true` only when the VUI signalled full range;
    /// the spec's inferred value when the video signal type is absent is 0
    /// (limited range), so this returns `false` then.
    pub fn full_range(&self) -> bool {
        self.video_signal.is_some_and(|vst| vst.full_range())
    }

    /// `video_format`, the signalled value or the inferred 5 when the video
    /// signal type is absent (H.264 §E.2.1).
    pub fn video_format(&self) -> VideoFormat {
        self.video_signal
            .map_or_else(VideoFormat::inferred, |vst| vst.video_format())
    }

    /// The colour description, the signalled triple or, when
    /// `colour_description_present_flag` is 0 (or the whole video signal type
    /// is absent), the spec's inferred primaries / transfer / matrix: 2/2/2
    /// (unspecified) for H.264/H.265, but 1/1/1 (BT.709) for MPEG-2
    /// ([`MPEG2_INFERRED_COLOUR_CODE`], H.262 §6.3.6). A player wanting to
    /// know whether the stream really said BT.709 reads
    /// [`video_signal`](Self::video_signal) instead.
    pub fn colour_description(&self) -> ColourDescription {
        self.video_signal
            .and_then(|vst| vst.colour())
            .unwrap_or(if self.mpeg2 {
                ColourDescription::from_wire(
                    MPEG2_INFERRED_COLOUR_CODE,
                    MPEG2_INFERRED_COLOUR_CODE,
                    MPEG2_INFERRED_COLOUR_CODE,
                )
            } else {
                ColourDescription::from_wire(
                    INFERRED_COLOUR_CODE,
                    INFERRED_COLOUR_CODE,
                    INFERRED_COLOUR_CODE,
                )
            })
    }
}

/// Which wrapper carried the AFD/bar data.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserDataCarriage {
    AvcSei,
    HevcPrefixSei,
    Mpeg2PictureUserData,
}

impl UserDataCarriage {
    pub fn name(&self) -> &'static str {
        match self {
            Self::AvcSei => "AVC SEI",
            Self::HevcPrefixSei => "HEVC prefix SEI",
            Self::Mpeg2PictureUserData => "MPEG-2 picture user data",
        }
    }
}
broadcast_core::impl_spec_display!(UserDataCarriage);

/// Picture-level "area of interest" state in force for an access unit.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveFormat {
    /// `None` when the last afd_data() had active_format_flag == 0, or no AFD
    /// has been seen.
    pub afd: Option<ActiveFormatDescription>,
    pub bar_data: Option<BarData>,
    pub carriage: UserDataCarriage,
}

impl ActiveFormat {
    /// Construct a picture-level state (additive Phase 3 constructor).
    pub fn new(
        afd: Option<ActiveFormatDescription>,
        bar_data: Option<BarData>,
        carriage: UserDataCarriage,
    ) -> Self {
        Self {
            afd,
            bar_data,
            carriage,
        }
    }
}

/// Persistence policy for AFD / bar data in H.264/H.265 (design §6.4).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AfdPersistence {
    /// Sticky until a new AFD message replaces it (default).
    #[default]
    UntilReplaced,
    /// Reset at a random-access (IDR/IRAP) AU that carries no AFD: afd becomes
    /// `Some(SameAsCodedFrame)` and bar data becomes `None` (ANSI/SCTE 128-1
    /// §8.2.3-8.2.4).
    ResetAtRandomAccess,
}

impl AfdPersistence {
    pub fn name(&self) -> &'static str {
        match self {
            Self::UntilReplaced => "until replaced",
            Self::ResetAtRandomAccess => "reset at random access",
        }
    }
}
broadcast_core::impl_spec_display!(AfdPersistence);

/// Carried twice, always behind `alloc::sync::Arc` because it is shared by
/// every picture of a configuration and never copied per sample:
/// [`Sample::display`](super::Sample::display) for the state in force at one
/// access unit, and [`DemuxEvent::DisplayChanged`](super::DemuxEvent::DisplayChanged)
/// announcing a new state before the first access unit it applies to.
///
/// Phase 2 populates [`geometry`](Self::geometry) only; picture-level AFD and
/// bar data are Phase 3 and land as additive fields.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    /// The sequence-level geometry of the SPS active at this access unit.
    /// `None` for a track with no parseable SPS.
    pub geometry: Option<DisplayGeometry>,
    /// Picture-level active format (AFD) and bar data in force at this access
    /// unit, or `None` when none has been signalled.
    pub active_format: Option<ActiveFormat>,
}

impl DisplayInfo {
    /// A state carrying only a sequence-level geometry.
    pub fn new(geometry: Option<DisplayGeometry>) -> Self {
        Self {
            geometry,
            active_format: None,
        }
    }

    /// Attach an [`ActiveFormat`] to this state (additive Phase 3 constructor).
    pub fn with_active_format(mut self, active_format: Option<ActiveFormat>) -> Self {
        self.active_format = active_format;
        self
    }

    /// The state of a [`TrackSpec`](super::TrackSpec)'s static
    /// [`display`](super::TrackSpec::display) geometry, or `None` when that
    /// spec has none (nothing to announce, so no per-AU state either).
    pub fn from_geometry(geometry: Option<DisplayGeometry>) -> Option<Self> {
        geometry.map(|g| Self::new(Some(g)))
    }

    /// The sample aspect ratio in force, if any.
    pub fn sample_aspect_ratio(&self) -> Option<SampleAspectRatio> {
        self.geometry.as_ref()?.typed_sample_aspect_ratio()
    }

    /// Which parts of `self` differ from `previous`.
    ///
    /// Comparison is by **parsed value** ([`PartialEq`]), never by the bytes
    /// a parameter set arrived in (design §6.1): an SPS re-sent with a level
    /// or HRD edit but identical geometry yields an all-false set, while a
    /// wire-form change of the SAR (e.g. idc 4 `16:11` to Extended_SAR
    /// `64:45`) always sets [`DisplayChangeSet::aspect_ratio`] even when the
    /// resulting display aspect ratio is unchanged.
    pub fn changes_from(&self, previous: Option<&Self>) -> DisplayChangeSet {
        let Some(previous) = previous else {
            return DisplayChangeSet {
                initial: true,
                ..DisplayChangeSet::default()
            };
        };
        if previous == self {
            return DisplayChangeSet::default();
        }
        let previous_display = previous;
        let (current, previous) = (self.geometry.as_ref(), previous_display.geometry.as_ref());
        DisplayChangeSet {
            initial: false,
            aspect_ratio: aspect_ratio_of(current) != aspect_ratio_of(previous),
            output_rect: output_rect_of(current) != output_rect_of(previous),
            video_signal: current.and_then(|g| g.video_signal)
                != previous.and_then(|g| g.video_signal),
            overscan: current.and_then(|g| g.overscan_appropriate)
                != previous.and_then(|g| g.overscan_appropriate),
            default_display_rect: default_display_rect_of(current)
                != default_display_rect_of(previous),
            afd: self.active_format.as_ref().map(|a| a.afd)
                != previous_display.active_format.as_ref().map(|a| a.afd),
            bar_data: self.active_format.as_ref().and_then(|a| a.bar_data.clone())
                != previous_display
                    .active_format
                    .as_ref()
                    .and_then(|a| a.bar_data.clone()),
        }
    }
}

/// The wire aspect ratio (idc and, for Extended_SAR, both terms) in force.
fn aspect_ratio_of(g: Option<&DisplayGeometry>) -> Option<AspectRatioIdc> {
    g.and_then(|g| g.aspect_ratio)
}

/// The displayed rectangle (output crop) of a geometry, or `None` when the
/// geometry itself is absent.
fn output_rect_of(g: Option<&DisplayGeometry>) -> Option<PictureRect> {
    g.map(|g| g.output_rect)
}

/// The (never-applied) H.265 default display window of a geometry.
fn default_display_rect_of(g: Option<&DisplayGeometry>) -> Option<Option<PictureRect>> {
    g.map(|g| g.default_display_rect)
}

/// Which parts of a [`DisplayInfo`] differ from the previous state for a
/// track (design §4, §6.1). Every field is an exact comparison of that part.
///
/// A field is set by any wire-form difference of the part, so a SAR change
/// that leaves the display aspect ratio identical (16:11 -> 64:45) still sets
/// [`aspect_ratio`](Self::aspect_ratio), and an SPS whose bytes changed but
/// whose parsed geometry did not sets nothing.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayChangeSet {
    /// The first state for this track (level-triggered start, design §5.2).
    pub initial: bool,
    /// [`AspectRatioIdc`] changed (idc, or the Extended_SAR terms).
    pub aspect_ratio: bool,
    /// The output rectangle changed (Phase 2: the coded size; cropping is a
    /// later phase).
    pub output_rect: bool,
    /// The (never-applied) H.265 default display window changed. Always
    /// `false` in Phase 2.
    pub default_display_rect: bool,
    /// The [`VideoSignalType`] changed (video format, full range, colour).
    pub video_signal: bool,
    /// [`DisplayGeometry::overscan_appropriate`] changed.
    pub overscan: bool,
    pub afd: bool,
    pub bar_data: bool,
}

impl DisplayChangeSet {
    /// The change set of a track's first state.
    pub fn initial() -> Self {
        Self {
            initial: true,
            ..Self::default()
        }
    }

    /// Whether any field changed at all.
    pub fn any(&self) -> bool {
        *self != Self::default()
    }

    /// A short, stable label of the changed parts, in field order
    /// (`"initial aspect_ratio"`), used by tests and logs.
    pub fn names(&self) -> alloc::string::String {
        let mut out = alloc::string::String::new();
        let mut push = |set: bool, name: &str| {
            if set {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(name);
            }
        };
        push(self.initial, "initial");
        push(self.aspect_ratio, "aspect_ratio");
        push(self.output_rect, "output_rect");
        push(self.default_display_rect, "default_display_rect");
        push(self.video_signal, "video_signal");
        push(self.overscan, "overscan");
        push(self.afd, "afd");
        push(self.bar_data, "bar_data");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::DataIdentity;

    #[test]
    fn codecs_without_an_sps_have_no_geometry() {
        let data = CodecConfig::Data {
            identity: DataIdentity::Ts {
                stream_type: 0x06,
                descriptors: alloc::vec::Vec::new(),
                carriage: crate::ir::DataCarriage::Pes,
            },
        };
        assert_eq!(DisplayGeometry::from_codec_config(&data), None);
    }
}
