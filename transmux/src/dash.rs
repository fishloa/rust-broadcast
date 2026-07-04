//! DASH `.mpd` Media Presentation Description generation — ISO/IEC 23009-1.
//!
//! The DASH sibling of [`crate::media::HlsPackager`]: [`DashPackager`] renders a
//! [`Media`] as an MPEG-DASH Media Presentation Description
//! (MPD) XML document. One CMAF segment set therefore yields both an HLS media
//! playlist and a DASH manifest describing the same tracks.
//!
//! # Structure (ISO/IEC 23009-1:2014)
//!
//! - **MPD** (§5.3.1) — the root element; namespace
//!   `urn:mpeg:dash:schema:mpd:2011`, a `profiles` attribute (§5.3.1.2), and a
//!   `type` of `static` (VOD) or `dynamic` (live).
//! - **Period** (§5.3.2) — a single period covering the presentation.
//! - **`AdaptationSet`** (§5.3.3) — one per media kind (video / audio), carrying
//!   the common `mimeType` (`video/mp4` / `audio/mp4`).
//! - **`Representation`** (§5.3.5) — one per [`Track`], with
//!   `@id`, `@bandwidth`, RFC 6381 `@codecs`, and per-kind geometry / audio
//!   parameters.
//! - **`SegmentTemplate`** (§5.3.9.4) — two mutually-exclusive addressing modes
//!   ([`Addressing`], §5.3.9.4.4 Table 16 / L1628): `$Number$` with a constant
//!   nominal `@duration`, or `$Time$` with an explicit child
//!   **`SegmentTimeline`** (§5.3.9.6) of `<S t= d= r=>` runs. Both carry
//!   `initialization` + `media` templates (`$RepresentationID$` plus the
//!   addressing identifier) and `timescale`.
//!
//! # Live (`dynamic`) presentations (§5.3.1.2 Table 3)
//!
//! [`DashPackager::dynamic`] switches `MPD@type` to `dynamic`; the live-only
//! attributes ([`DashPackager::availability_start_time`],
//! [`DashPackager::publish_time`], [`DashPackager::minimum_update_period`],
//! [`DashPackager::time_shift_buffer_depth`],
//! [`DashPackager::suggested_presentation_delay`]) are emitted verbatim
//! (caller-supplied ISO-8601 / xs:duration strings) when present, and omitted
//! otherwise. `mediaPresentationDuration` (VOD-only, §5.3.1.2) is never emitted
//! for a dynamic MPD.
//!
//! # AdaptationSet content (§5.3.3)
//!
//! Every regular `AdaptationSet` carries a `Role` (§5.8.5.5,
//! `urn:mpeg:dash:role:2011`, value `main`) and, when every `Representation` in
//! the set agrees on one, an inherited `@lang` — for a TS-sourced audio track,
//! resolved from its `ISO_639_language_descriptor` (ETSI EN 300 468 §6.2.19) in
//! [`crate::pipeline::TrackSpec::es_info_descriptors`]. Two opt-in extension
//! points round out the manifest: [`DashPackager::content_protection`] emits a
//! bare `<ContentProtection>` identification element per system (§5.8.4.1,
//! optionally `cenc:default_KID` per ISO/IEC 23001-7 — full CENC `pssh`
//! carriage is a separate epic), and [`DashPackager::inband_event_streams`]
//! emits `<InbandEventStream>` (§5.3.3 / §5.10.3.3) on the video
//! `AdaptationSet` for an inband `emsg` scheme the caller flags (mp4-emsg's
//! [`crate::EmsgBox`] already carries the wire box; this only advertises it).
//!
//! The MPD is emitted with a tiny hand-rolled XML writer; the crate stays
//! dependency-free (like `HlsPackager`).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::aac_asc::AudioSpecificConfig;
use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::pipeline::CodecConfig;
use crate::sps::rfc6381_avc1;
use broadcast_common::Parse;

/// DASH MPD namespace (ISO/IEC 23009-1 §5.3.1.2 — `urn:mpeg:dash:schema:mpd:2011`).
pub const MPD_NAMESPACE: &str = "urn:mpeg:dash:schema:mpd:2011";

/// Default DASH profile — ISO Base media file format live profile
/// (ISO/IEC 23009-1 §8.4).
pub const PROFILE_ISOFF_LIVE: &str = "urn:mpeg:dash:profile:isoff-live:2011";

/// `schemeIdUri` for the DASH audio-channel-configuration descriptor
/// (ISO/IEC 23009-1 §5.8.5.4 / 23001-8).
const AUDIO_CHANNEL_SCHEME: &str = "urn:mpeg:dash:23003:3:audio_channel_configuration:2011";

/// `schemeIdUri` for the DASH trick-mode `SupplementalProperty`
/// (ISO/IEC 23009-1 §5.8.5.8 / DASH-IF IOP §3.3.5).
///
/// A trick-mode `AdaptationSet` carries
/// `<SupplementalProperty schemeIdUri="urn:mpeg:dash:trickmode:2016" value="<id>"/>`
/// to declare that it is a trick-play rendition referencing the main video
/// `AdaptationSet` identified by `value`.
pub const TRICKMODE_SCHEME: &str = "urn:mpeg:dash:trickmode:2016";

/// MIME type carried on a video `AdaptationSet` (`video/mp4`).
const MIME_VIDEO: &str = "video/mp4";
/// MIME type carried on an audio `AdaptationSet` (`audio/mp4`).
const MIME_AUDIO: &str = "audio/mp4";

/// RFC 6381 `codecs` string for VP8 (WebM codec registration).
const CODECS_VP8: &str = "vp8";
/// RFC 6381 `codecs` string for Vorbis (WebM codec registration).
const CODECS_VORBIS: &str = "vorbis";

/// `schemeIdUri` for the DASH `Role` element (ISO/IEC 23009-1 §5.8.5.5, Table 24
/// — the `urn:mpeg:dash:role:2011` scheme).
const ROLE_SCHEME: &str = "urn:mpeg:dash:role:2011";
/// `Role@value` this packager emits on every regular `AdaptationSet`
/// (ISO/IEC 23009-1 §5.8.5.5 Table 24) — the IR has no alternate-audio /
/// commentary / dub track-role modelling yet, so every rendition is "main".
const ROLE_MAIN: &str = "main";

/// XML namespace declared on `MPD` when a [`ContentProtectionSystem`] carries a
/// `default_kid` (ISO/IEC 23001-7 Common Encryption, `cenc:default_KID`).
const CENC_NAMESPACE: &str = "urn:mpeg:cenc:2013";

/// `ISO_639_language_descriptor` tag — ETSI EN 300 468 §6.2.19.
const ISO_639_LANGUAGE_DESCRIPTOR_TAG: u8 = 0x0A;

/// Segment addressing mode for `SegmentTemplate` (ISO/IEC 23009-1 §5.3.9.4.4
/// Table 16 / §5.3.9.6) — the two modes are mutually exclusive on one template
/// (§5.3.9.4.4 L1628).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Addressing {
    /// `$Number$` substitution with a constant nominal `SegmentTemplate@duration`
    /// (§5.3.9.4.4): start of segment N = `(N - startNumber) * @duration`; only
    /// the last segment may have a different actual duration (L1688).
    #[default]
    Number,
    /// `$Time$` substitution with an explicit child `<SegmentTimeline>`
    /// (§5.3.9.6): each `<S t= d= r=>` gives the exact start time and duration
    /// of a run of segments, so segment durations need not be constant.
    Timeline,
}

impl Addressing {
    /// The spec-token label for this addressing mode.
    pub fn name(&self) -> &'static str {
        match self {
            Addressing::Number => "number",
            Addressing::Timeline => "timeline",
        }
    }
}

broadcast_common::impl_spec_display!(Addressing);

/// Per-`Representation` segment durations for `$Time$`/`SegmentTimeline`
/// addressing, and/or the `$Number$` nominal `@duration` — supplied by the
/// caller from its own segmentation (e.g. [`crate::segmenter::Segmenter`]):
/// one duration per emitted media segment, **in the representation's own
/// media `@timescale` ticks** (`TrackSpec::timescale`), in segment order.
///
/// A segment's `SegmentTimeline` start time (`S@t`, and the `$Time$`
/// substitution) is the cumulative sum of the *preceding* entries in this
/// list — exactly a segment's `tfdt.baseMediaDecodeTime`
/// (ISO/IEC 14496-12:2015 §8.8.12) whenever the representation's decode
/// timeline starts at zero, which every segmenter in this crate
/// ([`crate::segmenter::Segmenter`], [`crate::ll_dash::LlSegmenter`])
/// guarantees.
#[derive(Debug, Clone)]
pub struct TrackSegments {
    /// The [`crate::pipeline::TrackSpec::track_id`] these durations belong to.
    pub track_id: u32,
    /// Segment durations, in decode/segment order, in this track's timescale.
    pub durations: Vec<u64>,
}

/// A `ContentProtection` element to attach to every regular `AdaptationSet`
/// (ISO/IEC 23009-1 §5.8.4.1) — a **hook**, not full CENC: it renders the bare
/// identification element (`schemeIdUri` + optional `value` +
/// optional `cenc:default_KID`, ISO/IEC 23001-7 Common Encryption); carrying a
/// full `<cenc:pssh>` payload is a separate epic.
#[derive(Debug, Clone)]
pub struct ContentProtectionSystem {
    /// `ContentProtection@schemeIdUri` (ISO/IEC 23009-1 §5.8.4.1) — e.g. a DRM
    /// system ID URN (`urn:uuid:...`) or `urn:mpeg:dash:mp4protection:2011`
    /// for the generic CENC scheme signalling element.
    pub scheme_id_uri: String,
    /// Optional `ContentProtection@value` (scheme-defined, e.g. `"cenc"`).
    pub value: Option<String>,
    /// Optional default key ID (ISO/IEC 23001-7 `cenc:default_KID`), rendered
    /// as the canonical dashed hex UUID form.
    pub default_kid: Option<[u8; 16]>,
}

/// An `InbandEventStream` element (ISO/IEC 23009-1 §5.3.3 / §5.10.3.3) —
/// announces a `emsg` scheme/value the client should process inband. Emitted
/// on the video `AdaptationSet` (the common real-world placement for
/// SCTE-35/ID3 event boxes riding the video track's fragments; the IR has no
/// separate event-track modelling).
#[derive(Debug, Clone)]
pub struct InbandEventStream {
    /// `InbandEventStream@schemeIdUri` — must match the `emsg.scheme_id_uri`
    /// of the boxes it announces (DASH-IF IOP; ISO/IEC 23009-1 §5.10.3.3.4).
    pub scheme_id_uri: String,
    /// Optional `InbandEventStream@value`.
    pub value: Option<String>,
}

/// The media kind of a track's `AdaptationSet`.
///
/// Determines the `mimeType` and which geometry / audio attributes a
/// `Representation` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// Video track — `video/mp4`, carries `@width`/`@height`/`@frameRate`.
    Video,
    /// Audio track — `audio/mp4`, carries `@audioSamplingRate` + channels.
    Audio,
}

impl MediaKind {
    /// The DASH spec token for this kind (`"video"` / `"audio"`).
    pub fn name(&self) -> &'static str {
        match self {
            MediaKind::Video => "video",
            MediaKind::Audio => "audio",
        }
    }

    /// The `mimeType` value for an `AdaptationSet` of this kind.
    fn mime_type(&self) -> &'static str {
        match self {
            MediaKind::Video => MIME_VIDEO,
            MediaKind::Audio => MIME_AUDIO,
        }
    }

    fn of(config: &CodecConfig) -> Self {
        if is_audio(config) {
            MediaKind::Audio
        } else {
            MediaKind::Video
        }
    }
}

broadcast_common::impl_spec_display!(MediaKind);

/// True if `config` is an audio codec (mirrors `CodecConfig::is_audio`, which is
/// private to the pipeline module).
fn is_audio(config: &CodecConfig) -> bool {
    matches!(
        config,
        CodecConfig::Aac { .. }
            | CodecConfig::Ac3 { .. }
            | CodecConfig::Eac3 { .. }
            | CodecConfig::Opus { .. }
            | CodecConfig::Flac { .. }
            | CodecConfig::Ac4 { .. }
            | CodecConfig::MpegH { .. }
            | CodecConfig::Dts { .. }
            | CodecConfig::MpegAudio { .. }
            | CodecConfig::Vorbis { .. }
    )
}

/// A DASH trick-mode `AdaptationSet` descriptor — ISO/IEC 23009-1 §5.8.5.8.
///
/// A trick-mode adaptation set is a sparse, I-frame-only video rendition
/// intended for timeline scrubbing. It references the main video
/// `AdaptationSet` via a `SupplementalProperty`:
///
/// ```xml
/// <SupplementalProperty
///     schemeIdUri="urn:mpeg:dash:trickmode:2016"
///     value="<main_adaptation_set_id>"/>
/// ```
///
/// It also carries `maxPlayoutRate` and `codingDependency="false"` on the
/// `AdaptationSet` element (DASH-IF IOP §3.3.5).
///
/// To render a trick-mode set, pass one [`TrickModeAdaptationSet`] value in
/// [`DashPackager::trick_mode`].
#[derive(Debug, Clone)]
pub struct TrickModeAdaptationSet {
    /// The `@id` attribute of this trick-mode `AdaptationSet` (must be unique
    /// in the Period).
    pub id: String,
    /// The `@id` of the main video `AdaptationSet` this rendition references
    /// (the `value` attribute of `SupplementalProperty`).
    pub main_adaptation_set_id: String,
    /// The `maxPlayoutRate` attribute on the `AdaptationSet` (e.g. `8` for 8×
    /// fast-forward). Typically 2–16.
    pub max_playout_rate: u32,
    /// The trick-mode `Representation` — one track's parameters (codecs,
    /// bandwidth, resolution) as resolved by the caller.
    pub repr: TrickModeRepr,
}

/// Presentation parameters for a trick-mode `Representation`.
#[derive(Debug, Clone)]
pub struct TrickModeRepr {
    /// Representation `@id`.
    pub id: String,
    /// RFC 6381 codec string (e.g. `"avc1.64001e"`).
    pub codecs: String,
    /// Bandwidth in bits per second.
    pub bandwidth: u64,
    /// Coded width in pixels (`None` to omit).
    pub width: Option<u32>,
    /// Coded height in pixels (`None` to omit).
    pub height: Option<u32>,
    /// `SegmentTemplate@timescale`.
    pub timescale: u32,
    /// `SegmentTemplate@duration` (total sample-duration sum in timescale units).
    pub total_duration: u64,
}

/// Render an MPEG-DASH MPD (ISO/IEC 23009-1) describing a [`Media`].
///
/// Groups the media's tracks into one video and one audio
/// `AdaptationSet` (only kinds that have tracks are emitted) and emits a
/// `Representation` per track with a number-based
/// `SegmentTemplate`.
///
/// VOD (`type="static"`) is the default; set [`DashPackager::dynamic`] for a
/// live (`type="dynamic"`) manifest, optionally supplying
/// [`availability_start_time`](DashPackager::availability_start_time) and
/// [`minimum_update_period`](DashPackager::minimum_update_period) (both are
/// emitted verbatim as ISO-8601 / xs:duration strings when present, and omitted
/// otherwise — kept deliberately simple).
///
/// # Trick-mode signalling
///
/// Set [`DashPackager::trick_mode`] to emit an additional trick-mode
/// `AdaptationSet` alongside the regular video set. The trick-mode set carries
/// `<SupplementalProperty schemeIdUri="urn:mpeg:dash:trickmode:2016"
/// value="<id>"/>` (ISO/IEC 23009-1 §5.8.5.8) and `codingDependency="false"`.
#[derive(Debug, Clone)]
pub struct DashPackager {
    /// The MPD `profiles` attribute (ISO/IEC 23009-1 §5.3.1.2).
    pub profiles: String,
    /// If `true`, emit `type="dynamic"` (live); otherwise `type="static"` (VOD).
    pub dynamic: bool,
    /// Segment addressing mode (ISO/IEC 23009-1 §5.3.9.4.4 / §5.3.9.6). Default
    /// [`Addressing::Number`] — unchanged single-nominal-duration behaviour.
    /// [`Addressing::Timeline`] requires a matching, non-empty
    /// [`TrackSegments`] entry in [`Self::segments`] for every track.
    pub addressing: Addressing,
    /// Per-track segment durations (see [`TrackSegments`]). Used for
    /// `SegmentTimeline` generation under [`Addressing::Timeline`]; also
    /// overrides the [`Addressing::Number`] nominal `@duration` (the first
    /// entry) when present, instead of the whole-track total.
    pub segments: Vec<TrackSegments>,
    /// `SegmentTemplate@startNumber` (1-based, ISO/IEC 23009-1 §5.3.9.4.4).
    pub start_number: u64,
    /// `initialization` template (contains `$RepresentationID$`).
    pub init_template: String,
    /// `media` template for [`Addressing::Number`] (contains
    /// `$RepresentationID$` and `$Number$`).
    pub media_template: String,
    /// `media` template for [`Addressing::Timeline`] (contains
    /// `$RepresentationID$` and `$Time$`).
    pub media_template_time: String,
    /// Optional `MPD@availabilityStartTime` (ISO-8601 UTC), live only
    /// (ISO/IEC 23009-1 §5.3.1.2 Table 3).
    pub availability_start_time: Option<String>,
    /// Optional `MPD@publishTime` (ISO-8601 UTC), live only (§5.3.1.2 Table 3).
    pub publish_time: Option<String>,
    /// Optional `MPD@minimumUpdatePeriod` (xs:duration), live only
    /// (§5.3.1.2 Table 3).
    pub minimum_update_period: Option<String>,
    /// Optional `MPD@timeShiftBufferDepth` (xs:duration), live only — the
    /// depth of the live-edge time-shift buffer (§5.3.1.2 Table 3).
    pub time_shift_buffer_depth: Option<String>,
    /// Optional `MPD@suggestedPresentationDelay` (xs:duration), live only —
    /// the client's recommended distance behind the live edge (§5.3.1.2
    /// Table 3).
    pub suggested_presentation_delay: Option<String>,
    /// `ContentProtection` hooks (§5.8.4.1) emitted on every regular
    /// `AdaptationSet`. Empty by default (no protection signalled).
    pub content_protection: Vec<ContentProtectionSystem>,
    /// `InbandEventStream` elements (§5.3.3 / §5.10.3.3) emitted on the video
    /// `AdaptationSet`. Empty by default (no inband events signalled).
    pub inband_event_streams: Vec<InbandEventStream>,
    /// Optional trick-mode adaptation set (ISO/IEC 23009-1 §5.8.5.8).
    ///
    /// When `Some`, `package` emits an additional `AdaptationSet` after the
    /// regular video/audio sets, carrying the trick-mode descriptor.
    pub trick_mode: Option<TrickModeAdaptationSet>,
}

impl Default for DashPackager {
    fn default() -> Self {
        Self {
            profiles: PROFILE_ISOFF_LIVE.to_string(),
            dynamic: false,
            addressing: Addressing::Number,
            segments: Vec::new(),
            start_number: 1,
            init_template: String::from("init-stream$RepresentationID$.m4s"),
            media_template: String::from("chunk-stream$RepresentationID$-$Number$.m4s"),
            media_template_time: String::from("chunk-stream$RepresentationID$-$Time$.m4s"),
            availability_start_time: None,
            publish_time: None,
            minimum_update_period: None,
            time_shift_buffer_depth: None,
            suggested_presentation_delay: None,
            content_protection: Vec::new(),
            inband_event_streams: Vec::new(),
            trick_mode: None,
        }
    }
}

/// Derived per-track presentation parameters resolved from a [`CodecConfig`].
struct ReprInfo {
    id: String,
    kind: MediaKind,
    codecs: String,
    bandwidth: u64,
    timescale: u32,
    /// Sum of sample durations, in the track's timescale (SegmentTemplate@duration).
    total_duration: u64,
    /// Per-segment durations for this track, if the caller supplied a matching
    /// [`TrackSegments`] entry (see [`DashPackager::segments`]).
    segment_durations: Option<Vec<u64>>,
    /// `@lang` resolved from the track's `ISO_639_language_descriptor`
    /// (audio only — see [`lang_from_es_info`]).
    lang: Option<String>,
    // Video geometry.
    width: Option<u32>,
    height: Option<u32>,
    frame_rate: Option<String>,
    // Audio parameters.
    audio_sampling_rate: Option<u32>,
    audio_channels: Option<u16>,
}

impl DashPackager {
    /// Build a packager with a specific `profiles` attribute.
    pub fn with_profiles(profiles: impl Into<String>) -> Self {
        Self {
            profiles: profiles.into(),
            ..Self::default()
        }
    }

    /// Resolve the RFC 6381 codec string for a track (via the crate's own
    /// builders — never a hardcoded literal).
    fn codec_string(config: &CodecConfig) -> Result<String> {
        match config {
            CodecConfig::Avc { config, .. } => Ok(rfc6381_avc1(
                config.config.profile_indication,
                config.config.profile_compatibility,
                config.config.level_indication,
            )),
            CodecConfig::Hevc { config, .. } => Ok(config.config.rfc6381()),
            CodecConfig::Vvc { config, .. } => Ok(config.config.rfc6381()),
            CodecConfig::Aac { esds, .. } => {
                let asc = asc_from_esds(esds)?;
                Ok(asc.rfc6381())
            }
            CodecConfig::Ac3 { config, .. } => Ok(config.rfc6381().to_string()),
            CodecConfig::Eac3 { config, .. } => Ok(config.rfc6381().to_string()),
            CodecConfig::Opus { config, .. } => Ok(config.rfc6381().to_string()),
            CodecConfig::Flac { config, .. } => Ok(config.rfc6381().to_string()),
            CodecConfig::Ac4 { config, .. } => Ok(config.rfc6381().to_string()),
            CodecConfig::Av1 { config, .. } => Ok(config.rfc6381()),
            CodecConfig::Vp9 { config, .. } => Ok(config.rfc6381()),
            CodecConfig::MpegH { config, .. } => Ok(config.rfc6381()),
            CodecConfig::Dts { codec_fourcc, .. } => {
                Ok(crate::dts::DtsSpecificBox::rfc6381(codec_fourcc).to_string())
            }
            // RFC 6381 §3.3: MP4 registration uses the sample-entry FourCC plus
            // the ObjectTypeIndication (e.g. `mp4v.61`, `mp4a.6B`).
            CodecConfig::Mpeg2Video { esds, .. } => Ok(format!("mp4v.{:02X}", oti_of(esds))),
            CodecConfig::MpegAudio { esds, .. } => Ok(format!("mp4a.{:02X}", oti_of(esds))),
            // WebM-native codecs (RFC 6386 VP8 / Vorbis I); the WebM codec
            // registration uses the bare codec name.
            CodecConfig::Vp8 { .. } => Ok(CODECS_VP8.to_string()),
            CodecConfig::Vorbis { .. } => Ok(CODECS_VORBIS.to_string()),
            // Opaque PES data track: no RFC 6381 codec string can be derived
            // without decoding the payload (mirrors the fMP4 mux rejection).
            CodecConfig::Data { .. } => Err(Error::UnsupportedCodec { codec: "Data" }),
        }
    }

    /// Resolve every presentation parameter for one track.
    fn repr_info(&self, track: &Track) -> Result<ReprInfo> {
        let config = &track.spec.config;
        let timescale = track.spec.timescale.max(1);
        let total_duration: u64 = track.samples.iter().map(|s| s.duration as u64).sum();
        let total_bytes: u64 = track.samples.iter().map(|s| s.data.len() as u64).sum();

        // bandwidth = total coded bits / duration in seconds, ISO/IEC 23009-1
        // §5.3.5.2 (@bandwidth). Computed with integer round-to-nearest so no
        // std-only float intrinsic is needed in `no_std`:
        // round(total_bytes * 8 * timescale / total_duration). Guaranteed >= 1
        // so the MPD is always valid.
        let bandwidth = if total_duration > 0 {
            let bits_times_ts = total_bytes
                .saturating_mul(8)
                .saturating_mul(timescale as u64);
            div_round(bits_times_ts, total_duration)
        } else {
            0
        }
        .max(1);

        let kind = MediaKind::of(config);
        let codecs = Self::codec_string(config)?;

        let segment_durations = self
            .segments
            .iter()
            .find(|s| s.track_id == track.spec.track_id)
            .map(|s| s.durations.clone());

        // `@lang` (ISO/IEC 23009-1 §5.3.3.2 inherited attribute) — resolved from
        // the TS PMT ES_info descriptor loop (issue #582) for audio tracks only;
        // `transmux` does not otherwise decode SI descriptors.
        let lang = if kind == MediaKind::Audio {
            lang_from_es_info(&track.spec.es_info_descriptors)
        } else {
            None
        };

        let mut info = ReprInfo {
            id: track.spec.track_id.to_string(),
            kind,
            codecs,
            bandwidth,
            timescale,
            total_duration,
            segment_durations,
            lang,
            width: None,
            height: None,
            frame_rate: None,
            audio_sampling_rate: None,
            audio_channels: None,
        };

        match config {
            CodecConfig::Avc {
                config: avc,
                width,
                height,
            } => {
                // Prefer the SPS-decoded dimensions (the authoritative coded
                // geometry); fall back to the sample-entry values.
                let (w, h) = avc
                    .config
                    .sps
                    .first()
                    .and_then(|sps| sps.decode().ok())
                    .map(|i| (i.width, i.height))
                    .unwrap_or((*width as u32, *height as u32));
                info.width = Some(w);
                info.height = Some(h);
                info.frame_rate = frame_rate_from_samples(&track.samples, info.timescale);
            }
            CodecConfig::Hevc { width, height, .. }
            | CodecConfig::Vvc { width, height, .. }
            | CodecConfig::Av1 { width, height, .. }
            | CodecConfig::Vp9 { width, height, .. }
            | CodecConfig::Vp8 { width, height, .. }
            | CodecConfig::Mpeg2Video { width, height, .. } => {
                info.width = Some(*width as u32);
                info.height = Some(*height as u32);
                info.frame_rate = frame_rate_from_samples(&track.samples, info.timescale);
            }
            CodecConfig::Aac {
                sample_rate,
                channel_count,
                esds,
                ..
            } => {
                // Prefer the ASC-decoded sampling rate; fall back to the entry.
                let asc = asc_from_esds(esds).ok();
                info.audio_sampling_rate = Some(
                    asc.as_ref()
                        .and_then(asc_sampling_rate)
                        .unwrap_or(*sample_rate),
                );
                info.audio_channels = Some(*channel_count);
            }
            CodecConfig::Ac3 {
                sample_rate,
                channel_count,
                ..
            }
            | CodecConfig::Eac3 {
                sample_rate,
                channel_count,
                ..
            }
            | CodecConfig::Opus {
                sample_rate,
                channel_count,
                ..
            }
            | CodecConfig::Flac {
                sample_rate,
                channel_count,
                ..
            }
            | CodecConfig::Ac4 {
                sample_rate,
                channel_count,
                ..
            }
            | CodecConfig::MpegH {
                sample_rate,
                channel_count,
                ..
            }
            | CodecConfig::Dts {
                sample_rate,
                channel_count,
                ..
            }
            | CodecConfig::MpegAudio {
                sample_rate,
                channel_count,
                ..
            } => {
                info.audio_sampling_rate = Some(*sample_rate);
                info.audio_channels = Some(*channel_count);
            }
            CodecConfig::Vorbis {
                sample_rate,
                channels,
                ..
            } => {
                info.audio_sampling_rate = Some(*sample_rate);
                info.audio_channels = Some(*channels);
            }
            // Opaque PES data track: no video/audio presentation parameters to
            // resolve (unreachable in practice — `codec_string` above already
            // errors for `Data`, short-circuiting this function via `?`).
            CodecConfig::Data { .. } => {}
        }

        Ok(info)
    }

    /// Render the MPD XML for the resolved representations.
    fn render(&self, reprs: &[ReprInfo]) -> String {
        let mut w = XmlWriter::new();
        w.declaration();

        // --- MPD (root) ---
        let mut mpd_attrs = alloc::vec![
            ("xmlns", MPD_NAMESPACE.to_string()),
            ("profiles", self.profiles.clone()),
            (
                "type",
                if self.dynamic { "dynamic" } else { "static" }.to_string(),
            ),
            ("minBufferTime", "PT2.0S".to_string()),
        ];
        // `xmlns:cenc` (ISO/IEC 23001-7) only when a ContentProtection system
        // actually carries a `default_KID` in that namespace.
        if self
            .content_protection
            .iter()
            .any(|c| c.default_kid.is_some())
        {
            mpd_attrs.push(("xmlns:cenc", CENC_NAMESPACE.to_string()));
        }
        if self.dynamic {
            // Live-only attributes (ISO/IEC 23009-1 §5.3.1.2 Table 3), each
            // emitted verbatim (caller-supplied ISO-8601 / xs:duration) when
            // present and omitted otherwise.
            if let Some(ast) = &self.availability_start_time {
                mpd_attrs.push(("availabilityStartTime", ast.clone()));
            }
            if let Some(pt) = &self.publish_time {
                mpd_attrs.push(("publishTime", pt.clone()));
            }
            if let Some(mup) = &self.minimum_update_period {
                mpd_attrs.push(("minimumUpdatePeriod", mup.clone()));
            }
            if let Some(tsbd) = &self.time_shift_buffer_depth {
                mpd_attrs.push(("timeShiftBufferDepth", tsbd.clone()));
            }
            if let Some(spd) = &self.suggested_presentation_delay {
                mpd_attrs.push(("suggestedPresentationDelay", spd.clone()));
            }
        } else {
            // VOD: advertise the presentation duration (longest track), in
            // tenths of a second computed with integer math (`no_std`-safe).
            let max_tenths = reprs
                .iter()
                .map(|r| div_round(r.total_duration.saturating_mul(10), r.timescale as u64))
                .max()
                .unwrap_or(0);
            mpd_attrs.push(("mediaPresentationDuration", xs_duration_tenths(max_tenths)));
        }
        w.open("MPD", &mpd_attrs);

        // --- Period ---
        w.open(
            "Period",
            &[("id", "0".to_string()), ("start", "PT0.0S".to_string())],
        );

        for kind in [MediaKind::Video, MediaKind::Audio] {
            let set: Vec<&ReprInfo> = reprs.iter().filter(|r| r.kind == kind).collect();
            if set.is_empty() {
                continue;
            }
            self.write_adaptation_set(&mut w, kind, &set);
        }

        // Trick-mode AdaptationSet — ISO/IEC 23009-1 §5.8.5.8 / DASH-IF IOP §3.3.5.
        if let Some(tm) = &self.trick_mode {
            self.write_trick_adaptation_set(&mut w, tm);
        }

        w.close("Period");
        w.close("MPD");
        w.finish()
    }

    fn write_adaptation_set(&self, w: &mut XmlWriter, kind: MediaKind, set: &[&ReprInfo]) {
        let mut attrs = alloc::vec![
            ("contentType", kind.name().to_string()),
            ("mimeType", kind.mime_type().to_string()),
            ("segmentAlignment", "true".to_string()),
            ("startWithSAP", "1".to_string()),
        ];
        // `@lang` (ISO/IEC 23009-1 §5.3.3.2, an inherited xml:lang-style
        // attribute) — only when every Representation in the set agrees.
        if let Some(lang) = common_lang(set) {
            attrs.push(("lang", lang));
        }
        w.open("AdaptationSet", &attrs);

        // Role (ISO/IEC 23009-1 §5.8.5.5, Table 24) — every rendition here is
        // "main" (no alternate/commentary modelling in the IR).
        w.empty(
            "Role",
            &[
                ("schemeIdUri", ROLE_SCHEME.to_string()),
                ("value", ROLE_MAIN.to_string()),
            ],
        );

        // ContentProtection hooks (§5.8.4.1).
        for cp in &self.content_protection {
            write_content_protection(w, cp);
        }

        // InbandEventStream (§5.3.3 / §5.10.3.3) — advertised on the video
        // AdaptationSet only (see module docs for the placement rationale).
        if kind == MediaKind::Video {
            for ies in &self.inband_event_streams {
                let mut ies_attrs = alloc::vec![("schemeIdUri", ies.scheme_id_uri.clone())];
                if let Some(v) = &ies.value {
                    ies_attrs.push(("value", v.clone()));
                }
                w.empty("InbandEventStream", &ies_attrs);
            }
        }

        for r in set {
            let mut rattrs = alloc::vec![
                ("id", r.id.clone()),
                ("mimeType", kind.mime_type().to_string()),
                ("codecs", r.codecs.clone()),
                ("bandwidth", r.bandwidth.to_string()),
            ];
            if let (Some(wd), Some(ht)) = (r.width, r.height) {
                rattrs.push(("width", wd.to_string()));
                rattrs.push(("height", ht.to_string()));
            }
            if let Some(fr) = &r.frame_rate {
                rattrs.push(("frameRate", fr.clone()));
            }
            if let Some(sr) = r.audio_sampling_rate {
                rattrs.push(("audioSamplingRate", sr.to_string()));
            }
            w.open("Representation", &rattrs);

            if kind == MediaKind::Audio {
                if let Some(ch) = r.audio_channels {
                    w.empty(
                        "AudioChannelConfiguration",
                        &[
                            ("schemeIdUri", AUDIO_CHANNEL_SCHEME.to_string()),
                            ("value", ch.to_string()),
                        ],
                    );
                }
            }

            self.write_segment_template(w, r);

            w.close("Representation");
        }

        w.close("AdaptationSet");
    }

    /// Write one Representation's `SegmentTemplate` (ISO/IEC 23009-1
    /// §5.3.9.4.4), dispatching on [`Self::addressing`]:
    ///
    /// - [`Addressing::Number`] — `$Number$`, a self-closing element carrying
    ///   a single nominal `@duration` (the first entry of
    ///   [`ReprInfo::segment_durations`] when supplied, else the whole-track
    ///   total — the packager's original single-segment behaviour).
    /// - [`Addressing::Timeline`] — `$Time$`, with a child `<SegmentTimeline>`
    ///   (§5.3.9.6) built from [`ReprInfo::segment_durations`] (required; see
    ///   [`Package::package`](broadcast_common::Package::package) validation).
    ///   A multi-segment Representation carries `@duration` XOR
    ///   `SegmentTimeline`, never both (§5.3.9.2.2 L1381), so `@duration` is
    ///   omitted here.
    fn write_segment_template(&self, w: &mut XmlWriter, r: &ReprInfo) {
        match self.addressing {
            Addressing::Number => {
                let duration = match &r.segment_durations {
                    Some(durs) if !durs.is_empty() => durs[0],
                    _ => r.total_duration,
                };
                w.empty(
                    "SegmentTemplate",
                    &[
                        ("timescale", r.timescale.to_string()),
                        ("duration", duration.to_string()),
                        ("startNumber", self.start_number.to_string()),
                        ("initialization", self.init_template.clone()),
                        ("media", self.media_template.clone()),
                    ],
                );
            }
            Addressing::Timeline => {
                // Validated non-empty in `package()` before `render()` runs.
                let durations = r
                    .segment_durations
                    .as_ref()
                    .expect("Addressing::Timeline requires segment_durations (validated earlier)");
                w.open(
                    "SegmentTemplate",
                    &[
                        ("timescale", r.timescale.to_string()),
                        ("startNumber", self.start_number.to_string()),
                        ("initialization", self.init_template.clone()),
                        ("media", self.media_template_time.clone()),
                    ],
                );
                write_segment_timeline(w, durations);
                w.close("SegmentTemplate");
            }
        }
    }

    /// Write a trick-mode `AdaptationSet` — ISO/IEC 23009-1 §5.8.5.8.
    ///
    /// Emits:
    /// ```xml
    /// <AdaptationSet id="<id>" contentType="video" mimeType="video/mp4"
    ///                maxPlayoutRate="<n>" codingDependency="false">
    ///   <SupplementalProperty
    ///       schemeIdUri="urn:mpeg:dash:trickmode:2016"
    ///       value="<main_id>"/>
    ///   <Representation …/>
    ///   <SegmentTemplate …/>
    /// </AdaptationSet>
    /// ```
    fn write_trick_adaptation_set(&self, w: &mut XmlWriter, tm: &TrickModeAdaptationSet) {
        let mut as_attrs = alloc::vec![
            ("id", tm.id.clone()),
            ("contentType", "video".to_string()),
            ("mimeType", MIME_VIDEO.to_string()),
            ("maxPlayoutRate", tm.max_playout_rate.to_string()),
            ("codingDependency", "false".to_string()),
        ];
        // segmentAlignment not required by spec for trick-mode but keep consistent.
        as_attrs.push(("segmentAlignment", "true".to_string()));
        w.open("AdaptationSet", &as_attrs);

        // SupplementalProperty declaring the trick-mode relationship.
        // schemeIdUri = TRICKMODE_SCHEME (urn:mpeg:dash:trickmode:2016),
        // value = the @id of the main video AdaptationSet.
        w.empty(
            "SupplementalProperty",
            &[
                ("schemeIdUri", TRICKMODE_SCHEME.to_string()),
                ("value", tm.main_adaptation_set_id.clone()),
            ],
        );

        let r = &tm.repr;
        let mut rattrs = alloc::vec![
            ("id", r.id.clone()),
            ("mimeType", MIME_VIDEO.to_string()),
            ("codecs", r.codecs.clone()),
            ("bandwidth", r.bandwidth.to_string()),
        ];
        if let (Some(w_px), Some(h_px)) = (r.width, r.height) {
            rattrs.push(("width", w_px.to_string()));
            rattrs.push(("height", h_px.to_string()));
        }
        w.open("Representation", &rattrs);
        w.empty(
            "SegmentTemplate",
            &[
                ("timescale", r.timescale.to_string()),
                ("duration", r.total_duration.to_string()),
                ("startNumber", self.start_number.to_string()),
                ("initialization", self.init_template.clone()),
                ("media", self.media_template.clone()),
            ],
        );
        w.close("Representation");

        w.close("AdaptationSet");
    }
}

impl broadcast_common::Package for DashPackager {
    type Media = Media;
    type Output = String;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<String> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }
        let mut reprs = Vec::with_capacity(media.tracks.len());
        for t in &media.tracks {
            reprs.push(self.repr_info(t)?);
        }
        // Addressing::Timeline needs an explicit, non-empty duration list per
        // track (ISO/IEC 23009-1 §5.3.9.6) — there is no whole-track fallback
        // for $Time$/SegmentTimeline the way there is for $Number$.
        if self.addressing == Addressing::Timeline {
            for r in &reprs {
                if r.segment_durations.as_ref().is_none_or(Vec::is_empty) {
                    return Err(Error::InvalidInput(
                        "Addressing::Timeline requires a non-empty TrackSegments entry \
                         in DashPackager::segments for every track",
                    ));
                }
            }
        }
        Ok(self.render(&reprs))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract and parse the `AudioSpecificConfig` from an `esds` box.
fn asc_from_esds(esds: &crate::mp4esds::EsdsBox) -> Result<AudioSpecificConfig> {
    let dsi = esds
        .es_descriptor
        .decoder_config
        .as_ref()
        .and_then(|dc| dc.decoder_specific_info.as_ref())
        .ok_or(Error::UnexpectedBox {
            expected: "DecoderSpecificInfo (AudioSpecificConfig) in esds",
        })?;
    AudioSpecificConfig::parse(&dsi.data)
}

/// The `objectTypeIndication` carried in an `esds` (0 if the DecoderConfig is
/// absent) — used to build the RFC 6381 `mp4v.<OTI>` / `mp4a.<OTI>` codec string.
fn oti_of(esds: &crate::mp4esds::EsdsBox) -> u8 {
    esds.es_descriptor
        .decoder_config
        .as_ref()
        .map_or(0, |dc| dc.object_type_indication.0)
}

/// The effective sampling rate from a decoded ASC (explicit rate if present,
/// else the rate for the `samplingFrequencyIndex`, ISO/IEC 14496-3 Table 1.10).
fn asc_sampling_rate(asc: &AudioSpecificConfig) -> Option<u32> {
    if let Some(fs) = asc.sampling_frequency {
        return Some(fs);
    }
    // Table 1.10 (samplingFrequencyIndex → Hz); `Reserved`/`Escape` → None.
    const RATES: [u32; 13] = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
    ];
    RATES
        .get(asc.sampling_frequency_index.raw() as usize)
        .copied()
}

/// Derive a DASH `@frameRate` (`num/den`) from the video samples' durations.
///
/// Uses the (integer) average sample duration in the media timescale; returns
/// `timescale/avg_duration` reduced by the GCD. `None` if there is not enough
/// timing to compute a rate.
fn frame_rate_from_samples(samples: &[crate::pipeline::Sample], timescale: u32) -> Option<String> {
    if samples.is_empty() {
        return None;
    }
    let total: u64 = samples.iter().map(|s| s.duration as u64).sum();
    if total == 0 {
        return None;
    }
    let avg = total / samples.len() as u64;
    if avg == 0 {
        return None;
    }
    let num = timescale as u64;
    let den = avg;
    let g = gcd(num, den);
    Some(format!("{}/{}", num / g, den / g))
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

/// Divide `num / den` rounding to the nearest integer (ties up). `den` is
/// assumed non-zero by the caller.
fn div_round(num: u64, den: u64) -> u64 {
    (num + den / 2) / den
}

/// Format a duration given in tenths of a second as an xs:duration
/// (`PT<sec>.<tenth>S`, one decimal place; integer-only for `no_std`).
fn xs_duration_tenths(tenths: u64) -> String {
    format!("PT{}.{}S", tenths / 10, tenths % 10)
}

/// Extract the first `ISO_639_language_code` from a track's raw PMT ES_info
/// descriptor loop (verbatim TLV bytes carried on
/// [`crate::pipeline::TrackSpec::es_info_descriptors`], issue #582), by
/// walking the descriptor loop for tag `0x0A`
/// (`ISO_639_language_descriptor`, ETSI EN 300 468 §6.2.19: repeated
/// `ISO_639_language_code` (3 bytes) + `audio_type` (1 byte)). Returns `None`
/// for a non-TS source (empty descriptor bytes) or when no such descriptor is
/// present — `transmux` does not otherwise decode SI descriptors (that's
/// `dvb-si`'s job); this is the one field `@lang` needs.
fn lang_from_es_info(descriptors: &[u8]) -> Option<String> {
    let mut i = 0usize;
    while i + 2 <= descriptors.len() {
        let tag = descriptors[i];
        let len = descriptors[i + 1] as usize;
        let body_start = i + 2;
        let body_end = body_start + len;
        if body_end > descriptors.len() {
            break;
        }
        if tag == ISO_639_LANGUAGE_DESCRIPTOR_TAG && len >= 4 {
            let code = &descriptors[body_start..body_start + 3];
            if code.iter().all(u8::is_ascii_alphabetic) {
                if let Ok(s) = core::str::from_utf8(code) {
                    return Some(s.to_ascii_lowercase());
                }
            }
        }
        i = body_end;
    }
    None
}

/// The `@lang` shared by every Representation in `set`, or `None` if they
/// disagree (mixed-language grouping) or none carry one.
fn common_lang(set: &[&ReprInfo]) -> Option<String> {
    let first = set.first()?.lang.clone()?;
    if set
        .iter()
        .all(|r| r.lang.as_deref() == Some(first.as_str()))
    {
        Some(first)
    } else {
        None
    }
}

/// Write one `ContentProtection` element (ISO/IEC 23009-1 §5.8.4.1).
fn write_content_protection(w: &mut XmlWriter, cp: &ContentProtectionSystem) {
    let mut attrs = alloc::vec![("schemeIdUri", cp.scheme_id_uri.clone())];
    if let Some(v) = &cp.value {
        attrs.push(("value", v.clone()));
    }
    if let Some(kid) = &cp.default_kid {
        attrs.push(("cenc:default_KID", format_kid(kid)));
    }
    w.empty("ContentProtection", &attrs);
}

/// Format a 16-byte key ID as the canonical dashed-hex UUID form
/// (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, ISO/IEC 23001-7 `cenc:default_KID`).
fn format_kid(kid: &[u8; 16]) -> String {
    let hex = hex_lower(kid);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Lowercase hex encoding of `bytes`.
fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Write a `<SegmentTimeline>` (ISO/IEC 23009-1 §5.3.9.6) from a flat list of
/// per-segment durations (in the representation's own `@timescale`), in
/// segment order, starting at `t=0` (the packager's decode timelines all
/// start at zero — see [`TrackSegments`]).
///
/// Consecutive equal durations are run-length-encoded into a single `<S>`
/// with `@r` = repeat count minus one (§5.3.9.6 L1746: "`S` = a run of
/// contiguous equal-duration segments"). `@t` is emitted only on the first
/// `<S>` — every later run's start time is the spec-default derivation
/// (`prev S@t + prev@d * (prev@r+1)`, L1791), which this list satisfies by
/// construction (no gaps/discontinuities are modelled here).
fn write_segment_timeline(w: &mut XmlWriter, durations: &[u64]) {
    w.open("SegmentTimeline", &[]);
    let mut t: u64 = 0;
    let mut idx = 0usize;
    let mut first = true;
    while idx < durations.len() {
        let d = durations[idx];
        let mut run = 1usize;
        while idx + run < durations.len() && durations[idx + run] == d {
            run += 1;
        }
        let mut attrs: Vec<(&str, String)> = Vec::new();
        if first {
            attrs.push(("t", t.to_string()));
            first = false;
        }
        attrs.push(("d", d.to_string()));
        if run > 1 {
            attrs.push(("r", (run - 1).to_string()));
        }
        w.empty("S", &attrs);
        t += d * run as u64;
        idx += run;
    }
    w.close("SegmentTimeline");
}

// ---------------------------------------------------------------------------
// Tiny XML writer — no external dependency (dep-free like HlsPackager).
// ---------------------------------------------------------------------------

/// A minimal, indentation-aware XML element writer.
///
/// Escapes attribute values per XML 1.0 §2.4; emits `<?xml ...?>` then nested
/// open/close/empty elements. Not a general-purpose serializer — just enough to
/// render the MPD structure this module produces.
struct XmlWriter {
    buf: String,
    depth: usize,
}

impl XmlWriter {
    fn new() -> Self {
        Self {
            buf: String::new(),
            depth: 0,
        }
    }

    fn declaration(&mut self) {
        self.buf
            .push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    }

    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.buf.push_str("  ");
        }
    }

    fn attrs(&mut self, attrs: &[(&str, String)]) {
        for (k, v) in attrs {
            self.buf.push(' ');
            self.buf.push_str(k);
            self.buf.push_str("=\"");
            escape_into(&mut self.buf, v);
            self.buf.push('"');
        }
    }

    fn open(&mut self, name: &str, attrs: &[(&str, String)]) {
        self.indent();
        self.buf.push('<');
        self.buf.push_str(name);
        self.attrs(attrs);
        self.buf.push_str(">\n");
        self.depth += 1;
    }

    fn empty(&mut self, name: &str, attrs: &[(&str, String)]) {
        self.indent();
        self.buf.push('<');
        self.buf.push_str(name);
        self.attrs(attrs);
        self.buf.push_str("/>\n");
    }

    fn close(&mut self, name: &str) {
        self.depth = self.depth.saturating_sub(1);
        self.indent();
        self.buf.push_str("</");
        self.buf.push_str(name);
        self.buf.push_str(">\n");
    }

    fn finish(self) -> String {
        self.buf
    }
}

/// Escape a string for use in an XML attribute value (XML 1.0 §2.4).
fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
}
