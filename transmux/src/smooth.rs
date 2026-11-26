//! Microsoft Smooth Streaming (ISM/PIFF) output — [MS-SSTR].
//!
//! The Smooth-Streaming sibling of [`crate::dash::DashPackager`] and
//! [`crate::media::HlsPackager`]: [`SmoothSegmenter`] cuts a track stream into
//! Smooth **fragment** responses, each a fragmented-MP4 `moof`+`mdat` carrying
//! the Smooth-specific `tfxd` `uuid` box, and [`SmoothPackager`] renders the
//! Smooth **client Manifest** (XML) from the segment summaries those fragments
//! reported — it never sees a sample.
//!
//! # Structure ([MS-SSTR]; see `private/specs-verified/mpeg/transmux/smooth/ms-sstr.md`)
//!
//! - **Client Manifest** (§2.2.2): `SmoothStreamingMedia`(`MajorVersion=2`,
//!   `MinorVersion=0`, `Duration`, `TimeScale=10000000`) → one `StreamIndex`
//!   per media kind (`Type="video"`/`"audio"`, `Chunks`, `QualityLevels`,
//!   `Url`) → a `QualityLevel` (§2.2.2.5: `Index`, `Bitrate`, `FourCC`,
//!   `CodecPrivateData`, geometry / audio params) → one `c` element per
//!   fragment (§2.2.2.6: `d` = duration, `t` = absolute time on the first,
//!   `n` = ordinal).
//! - **Fragment response** (§2.2.4): `moof`(`mfhd` `traf`(`tfhd` `trun`
//!   **`tfxd`**)) `mdat`. The `tfxd` box (§2.2.4.4) is a `uuid` box with
//!   extended-type [`TFXD_UUID`] carrying `FragmentAbsoluteTime` +
//!   `FragmentDuration` (both in the manifest [`SMOOTH_TIMESCALE`]).
//! - **`tfrf`** look-ahead (§2.2.4.5) is live-only; omitted for VOD output.
//!
//! The standard `mfhd`/`tfhd`/`trun`/`mdat` boxes are the same fMP4 framing
//! (ISO/IEC 14496-12:2026 §8.8) the crate already builds; [`SmoothSegmenter`]
//! cuts each track keyframe-aligned (the [`crate::segmenter::Segmenter`] cut
//! rule applied per track, MS-SSTR fragments being per-stream) and injects the
//! `tfxd` `uuid` box into each fragment's `traf`.
//!
//! FourCC / `CodecPrivateData` (§2.2.2.5):
//! - Video `FourCC="H264"`: `CodecPrivateData` = the hex of SPS+PPS as
//!   start-code-prefixed NAL units (`00000001 <sps> 00000001 <pps>`).
//! - Audio `FourCC="AACL"`: `CodecPrivateData` = the hex of the
//!   AudioSpecificConfig; `AudioTag="255"` (raw AAC).
//!
//! Like the DASH / HLS packagers, the manifest is emitted with a tiny
//! [`quick_xml::Writer`] (`std` feature; every attribute value and text node is
//! escaped by quick-xml). Integer arithmetic only.

use crate::TrackId;
use crate::nalu_ext::AvcSpsCodecInfo;
use crate::units::ROUNDING_DIVISOR;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use broadcast_core::error::InvalidValue;
use broadcast_core::len::{need_in, need_out};

use broadcast_common::{Demand, Stage, Timestamp};
use broadcast_core::timescale::{Rounding, Timescale, rescale_i64, rescale_u64};
use broadcast_core::{Parse, Serialize};

use crate::aac_asc::AudioSpecificConfig;
use crate::error::{Error, Result};
use crate::ir::{Sample, TrackSpec};
use crate::mux_stage::{MuxInput, MuxOutput, SegmentOut, SegmentSummary};
use crate::pipeline::CodecConfig;
use crate::segment_log::{SegmentSummaryLog, TrackSummary};
use crate::segmenter::MAX_PENDING_SAMPLES_PER_TRACK;
use broadcast_core::wire::{be_u32, be_u64};
use broadcast_smooth::{C, QualityLevel, SmoothManifest, StreamIndex};
use isobmff::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BoxHeader, BoxType, FULLBOX_EXTRA_SIZE, UUID,
    UUID_TYPE_SIZE,
};
use isobmff::movie_fragment::{MovieFragmentBox, MovieFragmentHeaderBox, TRAF, TrackFragmentBox};
use isobmff::segments::{MDAT, MediaDataBox, SegmentTypeBox, starts_with_moof};
use isobmff::writer::{RunSample, media_fragment_run};

/// The Smooth Streaming default manifest time scale — 10 MHz (100 ns ticks),
/// [MS-SSTR] §2.2.2 (`SmoothStreamingMedia@TimeScale`); the single definition
/// lives in `broadcast-smooth`, which owns the manifest syntax.
pub use broadcast_smooth::{SMOOTH_DEFAULT_TIMESCALE, SMOOTH_TIMESCALE};

/// The `tfxd` `uuid` extended-type — [MS-SSTR] §2.2.4.4
/// (`6d1d9b05-42d5-44e6-80e2-141daff757b2`).
pub const TFXD_UUID: [u8; UUID_TYPE_SIZE] = [
    0x6d, 0x1d, 0x9b, 0x05, 0x42, 0xd5, 0x44, 0xe6, 0x80, 0xe2, 0x14, 0x1d, 0xaf, 0xf7, 0x57, 0xb2,
];

/// Video FourCC in the Smooth `QualityLevel` (§2.2.2.5) — H.264/AVC.
pub const FOURCC_H264: &str = "H264";
/// Video FourCC alias in the Smooth `QualityLevel` (§2.2.2.5) — H.264/AVC is
/// spelled `AVC1` by some encoders (`H264` is the primary name).
pub const FOURCC_AVC1: &str = "AVC1";
/// Audio FourCC in the Smooth `QualityLevel` (§2.2.2.5) — AAC-LC.
pub const FOURCC_AACL: &str = "AACL";
/// Audio FourCC in the Smooth `QualityLevel` (§2.2.2.5) — HE-AAC.
pub const FOURCC_AACH: &str = "AACH";
/// Text FourCC in the Smooth `QualityLevel` for a TTML/IMSC track: the ISOBMFF
/// `stpp` sample-entry code. [MS-SSTR] §2.2.2.5 reserves no text FourCC and
/// allows "a vendor extension value ... registered with MPEG4-RA"; `stpp`
/// (ISO/IEC 14496-30) is such a registered code and names the sample format.
pub const FOURCC_STPP: &str = "stpp";
/// Text FourCC in the Smooth `QualityLevel` for a WebVTT track: the ISOBMFF
/// `wvtt` sample-entry code (ISO/IEC 14496-30), a registered MPEG4-RA code.
pub const FOURCC_WVTT: &str = "wvtt";
/// `StreamIndex@Subtype` this packager writes for a text stream: `SUBT`,
/// "Subtitles that are used for foreign-language audio" ([MS-SSTR] §2.2.2.3).
/// The IR does not say whether a track is a caption service for the deaf
/// (`CAPT`), so the subtitle category is used.
pub const SUBTYPE_SUBT: &str = "SUBT";

/// `AudioTag` for raw AAC in the Smooth `QualityLevel` (§2.2.2.5).
const AUDIO_TAG_AAC: u32 = 255;
/// The quality-level count every stream this packager emits advertises.
const STREAM_QUALITY_LEVELS: u32 = 1;
/// The 1-based ordinal the first stream of a `Type` gets (its `Name` is the
/// bare `Type`).
const FIRST_STREAM_ORDINAL: usize = 1;
/// Bits-per-sample advertised for AAC audio.
const AAC_BITS_PER_SAMPLE: u16 = 16;

/// Annex B start code prefixing each parameter-set NAL in `CodecPrivateData`.
const START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// Length of one `tfxd` 64-bit time field.
const TFXD_TIME_LEN: usize = core::mem::size_of::<u64>();

/// The `styp` brand emitted ahead of a self-contained Smooth fragment (kept
/// consistent with the crate's CMAF media segments).
const STYP_MAJOR_BRAND: [u8; 4] = *b"msdh";

/// The media kind of a Smooth `StreamIndex`.
///
/// Determines the `Type` attribute, the FourCC, and which
/// geometry / audio attributes the `QualityLevel` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SmoothStreamType {
    /// Video stream — `Type="video"`, `FourCC="H264"`.
    Video,
    /// Audio stream — `Type="audio"`, `FourCC="AACL"`.
    Audio,
    /// Text stream (subtitles) — `Type="text"`, `Subtype="SUBT"`,
    /// `FourCC="stpp"` / `"wvtt"`.
    Text,
}

impl SmoothStreamType {
    /// The [MS-SSTR] `StreamIndex@Type` token (`"video"` / `"audio"` / `"text"`).
    pub fn name(&self) -> &'static str {
        match self {
            SmoothStreamType::Video => "video",
            SmoothStreamType::Audio => "audio",
            SmoothStreamType::Text => "text",
        }
    }
}

broadcast_core::impl_spec_display!(SmoothStreamType);

// ---------------------------------------------------------------------------
// tfxd uuid box — [MS-SSTR] §2.2.4.4
// ---------------------------------------------------------------------------

/// The Smooth `TfxdBox` (§2.2.4.4): a `uuid` FullBox (version 1) carrying the
/// fragment's `FragmentAbsoluteTime` + `FragmentDuration`, both in the manifest
/// [`SMOOTH_TIMESCALE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TfxdBox {
    /// Fragment absolute (start) time, in [`SMOOTH_TIMESCALE`] ticks.
    pub fragment_absolute_time: u64,
    /// Fragment duration, in [`SMOOTH_TIMESCALE`] ticks.
    pub fragment_duration: u64,
}

impl TfxdBox {
    /// FullBox version — 1 selects the 64-bit time/duration fields (§2.2.4.4).
    const VERSION: u8 = 1;
    /// Payload after the FullBox header: two 64-bit fields.
    const PAYLOAD_LEN: usize = 16;

    /// Build a `tfxd` box for a fragment.
    pub fn new(fragment_absolute_time: u64, fragment_duration: u64) -> Self {
        Self {
            fragment_absolute_time,
            fragment_duration,
        }
    }

    /// Parse a `tfxd` `uuid` box *body* (the bytes after the 8-byte box header
    /// and the 16-byte `usertype`, i.e. the FullBox header + payload).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        // version(1) + flags(3) + AbsoluteTime(8) + Duration(8)
        let need = FULLBOX_EXTRA_SIZE + Self::PAYLOAD_LEN;
        need_in(body, need, "tfxd body")?;
        let p = &body[FULLBOX_EXTRA_SIZE..];
        let fragment_absolute_time = be_u64(p, 0, "tfxd body")?;
        let fragment_duration = be_u64(p, TFXD_TIME_LEN, "tfxd body")?;
        Ok(Self {
            fragment_absolute_time,
            fragment_duration,
        })
    }
}

impl Serialize for TfxdBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        // box header(8) + usertype(16) + FullBox version/flags(4) + payload(16).
        let hdr = BoxHeader::new(0, BoxType::from_bytes(UUID), Some(TFXD_UUID));
        hdr.serialized_len() + FULLBOX_EXTRA_SIZE + Self::PAYLOAD_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        // uuid box header (size, "uuid", usertype = TFXD_UUID).
        let hdr = BoxHeader::new(need as u64, BoxType::from_bytes(UUID), Some(TFXD_UUID));
        let mut c = hdr.serialize_into(buf)?;
        // FullBox header: version + 24-bit flags(0).
        let fullbox = [Self::VERSION, 0, 0, 0];
        buf[c..c + fullbox.len()].copy_from_slice(&fullbox);
        c += fullbox.len();
        buf[c..c + TFXD_TIME_LEN].copy_from_slice(&self.fragment_absolute_time.to_be_bytes());
        c += TFXD_TIME_LEN;
        buf[c..c + TFXD_TIME_LEN].copy_from_slice(&self.fragment_duration.to_be_bytes());
        c += TFXD_TIME_LEN;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SmoothPackager
// ---------------------------------------------------------------------------

/// Render a Microsoft Smooth Streaming client Manifest ([MS-SSTR] §2.2.2).
///
/// The manifest never sees a sample: `tracks` is the stream set and `log` the
/// [`SegmentSummary`]s [`SmoothSegmenter`] emitted for it, one `c` element per
/// fragment, `Bitrate` from the coded bytes over the fragment durations. VOD
/// only (no live `tfrf` look-ahead).
#[derive(Debug, Clone, Copy, Default)]
pub struct SmoothPackager;

/// Per-track presentation parameters resolved for the manifest.
struct StreamInfo {
    stream_type: SmoothStreamType,
    fourcc: &'static str,
    /// `CodecPrivateData` bytes (hex-encoded into the manifest).
    codec_private_data: Vec<u8>,
    bitrate: u64,
    /// Video geometry.
    width: Option<u32>,
    height: Option<u32>,
    /// Audio params.
    sampling_rate: Option<u32>,
    channels: Option<u16>,
    /// Number of `QualityLevel` elements the manifest advertises for the
    /// stream — one per track, the fragments of which are served at one rate.
    qualities: u32,
    /// The track's fragments (index into the output list, resolved at render).
    fragments: Vec<FragmentTiming>,
    /// Sum of fragment durations, in [`SMOOTH_TIMESCALE`] ticks.
    total_duration: u64,
}

/// Timing for one `c` element.
struct FragmentTiming {
    start_time: u64,
    duration: u64,
}

impl SmoothPackager {
    /// Build one Smooth fragment for a single track at an explicit
    /// smooth-timeline start (issue #1083, W13) — the live-origin path, which
    /// already knows the window's fragment boundaries.
    ///
    /// Unlike [`SmoothSegmenter`], this does not segment the track or derive
    /// the start from the track's own decode times: it emits the whole track's
    /// samples as a single `moof`+`tfxd`+`mdat` whose `FragmentAbsoluteTime` is
    /// exactly `start_smooth` (in [`SMOOTH_TIMESCALE`] ticks) and whose
    /// `FragmentDuration` is the sum of the samples' own durations. That lets
    /// a live packager whose manifest `c@t` advertises a cumulative start
    /// serve a fragment whose `tfxd` matches it — the requirement MS-SSTR
    /// §2.2.4 places on a fragment response, and which a per-stream origin
    /// cannot express.
    ///
    /// `track` must carry at least one sample; `sequence_number` is
    /// `mfhd.sequence_number` (1-based).
    ///
    /// # Errors
    /// [`Error::InvalidInput`] on a track with no samples, or a sample whose
    /// smooth-tick duration or composition offset overflows its serialized
    /// field.
    pub fn package_track_fragment(
        &self,
        spec: &TrackSpec,
        samples: &[Sample],
        sequence_number: u32,
        start_smooth: u64,
    ) -> Result<SegmentOut> {
        if samples.is_empty() {
            return Err(Error::InvalidInput(
                "cannot package a Smooth fragment from a track with no samples",
            ));
        }
        // Times relative to the track's own first sample: only the *durations*
        // and composition offsets are used from here (the fragment's absolute
        // start is the caller's `start_smooth`, not the derived one).
        let origin = SmoothOrigin::from_tracks([(spec.timescale, samples)]);
        let times = smooth_times(samples, origin, spec.timescale, None)?;
        let duration: u64 = times.iter().map(|t| u64::from(t.duration)).sum();
        let data = build_smooth_fragment(
            spec.track_id,
            sequence_number,
            start_smooth,
            duration,
            samples,
            &times,
        )?;
        Ok(SegmentOut {
            summary: SegmentSummary {
                track: spec.track_id,
                sequence: u64::from(sequence_number),
                start: start_smooth,
                duration,
                timescale: SMOOTH_DEFAULT_TIMESCALE.get(),
                bytes: samples.iter().map(|s| s.data.len() as u64).sum(),
                independent: samples.first().is_some_and(|s| s.flags.is_sync),
                discontinuous: false,
            },
            bytes: bytes::Bytes::from(data),
            tracks: alloc::vec![crate::mux_stage::TrackPart {
                track: spec.track_id,
                start: start_smooth,
                duration,
                timescale: SMOOTH_DEFAULT_TIMESCALE.get(),
                bytes: samples.iter().map(|s| s.data.len() as u64).sum(),
                samples: u32::try_from(samples.len()).unwrap_or(u32::MAX),
            }],
        })
    }

    /// Render the client Manifest for `tracks` from the fragments `log`
    /// recorded.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] when `tracks` is empty, or a track's codec has
    /// no Smooth description ([`resolve_codec`]).
    pub fn render(&self, tracks: &[TrackSpec], log: &SegmentSummaryLog) -> Result<String> {
        if tracks.is_empty() {
            return Err(Error::InvalidInput(
                "cannot render a manifest with no tracks",
            ));
        }
        let mut streams = Vec::with_capacity(tracks.len());
        for spec in tracks {
            streams.push(self.stream_info(spec, log.track(spec.track_id))?);
        }
        self.render_manifest(&streams)
    }

    /// One track's manifest parameters: its codec description from the spec,
    /// its `c` timeline and bitrate from the summaries.
    fn stream_info(
        &self,
        spec: &TrackSpec,
        summary: Option<TrackSummary<'_>>,
    ) -> Result<StreamInfo> {
        let codec = resolve_codec(&spec.config)?;
        let total_duration = summary.map_or(0, |s| s.duration);
        let total_bytes = summary.map_or(0, |s| s.bytes);
        let fragments: Vec<FragmentTiming> = summary
            .map(|s| {
                s.runs()
                    .flat_map(|run| {
                        (0..run.segments).map(move |index| FragmentTiming {
                            start_time: run
                                .start
                                .saturating_add(run.duration.saturating_mul(index)),
                            duration: run.duration,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // bitrate = total coded bits / duration in seconds (integer round);
        // checked_div guards the zero-duration case (→ 0).
        let bitrate = {
            let bits_ts = total_bytes
                .saturating_mul(u64::from(u8::BITS))
                .saturating_mul(SMOOTH_TIMESCALE);
            (bits_ts + total_duration / ROUNDING_DIVISOR)
                .checked_div(total_duration)
                .unwrap_or(0)
        }
        .max(1);

        Ok(StreamInfo {
            stream_type: codec.stream_type,
            fourcc: codec.fourcc,
            codec_private_data: codec.codec_private_data,
            bitrate,
            width: codec.width,
            height: codec.height,
            sampling_rate: codec.sampling_rate,
            channels: codec.channels,
            qualities: STREAM_QUALITY_LEVELS,
            fragments,
            total_duration,
        })
    }
}

impl SmoothPackager {
    /// Build the Smooth client Manifest for the resolved streams and render
    /// it with the model's serializer
    /// ([`broadcast_smooth::SmoothManifest::to_xml`]).
    fn render_manifest(&self, streams: &[StreamInfo]) -> Result<String> {
        let duration = streams.iter().map(|s| s.total_duration).max().unwrap_or(0);
        let mut seen: Vec<(SmoothStreamType, usize)> = Vec::new();
        let streams = streams
            .iter()
            .map(|stream| {
                let ordinal = match seen.iter_mut().find(|(k, _)| *k == stream.stream_type) {
                    Some((_, count)) => {
                        *count += 1;
                        *count
                    }
                    None => {
                        seen.push((stream.stream_type, FIRST_STREAM_ORDINAL));
                        FIRST_STREAM_ORDINAL
                    }
                };
                self.stream_manifest_index(stream, ordinal)
            })
            .collect();
        Ok(SmoothManifest::new(streams)
            .with_duration(duration)
            .to_xml()?)
    }

    /// The manifest `StreamIndex` for one resolved track; `ordinal` is its
    /// 1-based position among the streams of the same `Type`.
    fn stream_manifest_index(&self, stream: &StreamInfo, ordinal: usize) -> StreamIndex {
        let ty = stream.stream_type.name();
        let name = if ordinal == FIRST_STREAM_ORDINAL {
            ty.to_string()
        } else {
            format!("{ty}_{ordinal}")
        };
        let url = format!("QualityLevels({{bitrate}})/Fragments({name}={{start time}})");
        let subtype = if stream.stream_type == SmoothStreamType::Text {
            SUBTYPE_SUBT.to_string()
        } else {
            String::new()
        };
        let chunks = u32::try_from(stream.fragments.len()).unwrap_or(u32::MAX);
        let mut index = StreamIndex::new(
            manifest_stream_type(stream.stream_type),
            url,
            alloc::vec![quality_level(stream),],
        )
        .with_subtype(subtype)
        .with_counts(chunks, stream.qualities)
        .with_chunks(manifest_chunks(stream));
        if ordinal > FIRST_STREAM_ORDINAL {
            index = index.with_name(name);
        }
        index
    }
}

/// The `StreamIndex@Type` token for a packager stream kind.
fn manifest_stream_type(stream_type: SmoothStreamType) -> broadcast_smooth::StreamType {
    match stream_type {
        SmoothStreamType::Video => broadcast_smooth::StreamType::Video,
        SmoothStreamType::Audio => broadcast_smooth::StreamType::Audio,
        SmoothStreamType::Text => broadcast_smooth::StreamType::Text,
    }
}

/// The single `QualityLevel` a track's fragments are served at, carrying the
/// geometry (video) or sample parameters (audio) the IR knows.
fn quality_level(stream: &StreamInfo) -> QualityLevel {
    let level = QualityLevel::new(
        stream.fourcc,
        stream.bitrate,
        stream.codec_private_data.clone(),
    );
    let level = match (stream.width, stream.height) {
        (Some(width), Some(height)) => level.with_size(width, height),
        _ => level,
    };
    match (stream.sampling_rate, stream.channels) {
        (Some(rate), Some(channels)) if stream.stream_type == SmoothStreamType::Audio => {
            level.with_audio(rate, channels, AAC_BITS_PER_SAMPLE, 0, AUDIO_TAG_AAC)
        }
        _ => level,
    }
    .without_packet_size()
}

/// One `c` per fragment, numbered from 0 (`c@n`); `t` on the first only, `d`
/// always.
fn manifest_chunks(stream: &StreamInfo) -> Vec<C> {
    stream
        .fragments
        .iter()
        .enumerate()
        .map(|(i, fragment)| {
            let ordinal = u32::try_from(i).unwrap_or(u32::MAX);
            let chunk = C::new(fragment.duration).numbered(ordinal);
            if i == 0 {
                chunk.at(fragment.start_time)
            } else {
                chunk
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Codec resolution
// ---------------------------------------------------------------------------

/// A track's Smooth codec description, resolved once from its [`CodecConfig`]
/// by [`resolve_codec`] — the single source of the manifest's `FourCC`,
/// `CodecPrivateData` and per-kind parameters for both this crate's packager
/// and the live origin in `smooth-runtime`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SmoothCodec {
    /// Which `StreamIndex` the track belongs to.
    pub stream_type: SmoothStreamType,
    /// The `QualityLevel@FourCC` (§2.2.2.5).
    pub fourcc: &'static str,
    /// The `QualityLevel@CodecPrivateData` bytes, hex-encoded into the
    /// manifest by the writer.
    pub codec_private_data: Vec<u8>,
    /// `QualityLevel@MaxWidth`, video only — decoded from the SPS.
    pub width: Option<u32>,
    /// `QualityLevel@MaxHeight`, video only — decoded from the SPS.
    pub height: Option<u32>,
    /// `QualityLevel@SamplingRate`, audio only — the AudioSpecificConfig's
    /// effective rate, so an SBR stream advertises its doubled rate.
    pub sampling_rate: Option<u32>,
    /// `QualityLevel@Channels`, audio only.
    pub channels: Option<u16>,
}

/// The `CodecPrivateData` bytes for a video track: start-code-prefixed
/// SPS+PPS NAL units (§2.2.2.5).
fn avc_codec_private_data(config: &isobmff::avc_config::AVCConfigurationBox) -> Vec<u8> {
    let mut out = Vec::new();
    for sps in &config.config.sps {
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(&sps.0);
    }
    for pps in &config.config.pps {
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(&pps.0);
    }
    out
}

/// The `CodecPrivateData` bytes for an AAC track: the raw AudioSpecificConfig
/// carried in the `esds` (§2.2.2.5). An `esds` without a
/// DecoderSpecificInfo describes no decoder, so it is refused rather than
/// advertised as an empty `CodecPrivateData` a client cannot initialise from.
fn aac_codec_private_data(esds: &isobmff::mp4esds::EsdsBox) -> Result<Vec<u8>> {
    let dsi = esds.require_decoder_specific_info()?;
    Ok(dsi.to_vec())
}

/// Convert a tick count in `media_timescale` to [`SMOOTH_TIMESCALE`] ticks with
/// integer round-to-nearest.
pub fn to_smooth_ticks(ticks: u64, media_timescale: broadcast_core::timescale::Timescale) -> u64 {
    rescale_u64(
        ticks,
        media_timescale,
        SMOOTH_DEFAULT_TIMESCALE,
        Rounding::Nearest,
    )
}

/// Resolve the Smooth stream type, FourCC, `CodecPrivateData` and per-kind
/// parameters for a track's [`CodecConfig`] — the one codec resolver shared by
/// [`SmoothPackager`] and `smooth-runtime`'s live origin.
///
/// H.264 video takes its geometry from the first SPS (the coded dimensions,
/// authoritative even when the `CodecConfig` carries none — RTP/WHEP ingest
/// builds `CodecConfig::Avc` with width and height `0`), AAC audio its
/// `AudioSpecificConfig` and reported channel count, and a subtitle track
/// re-muxed from an ISOBMFF `stpp`/`wvtt` sample entry its sample-format FourCC.
/// Every other codec is [`Error::InvalidInput`].
pub fn resolve_codec(config: &CodecConfig) -> Result<SmoothCodec> {
    match config {
        CodecConfig::Avc { config, .. } => {
            let (width, height) = config
                .config
                .sps
                .first()
                .and_then(|sps| sps.decode().ok())
                .map(|info| (info.width(), info.height()))
                .ok_or(Error::InvalidInput(
                    "AVC track has no decodable SPS for Smooth geometry",
                ))?;
            Ok(SmoothCodec {
                stream_type: SmoothStreamType::Video,
                fourcc: FOURCC_H264,
                codec_private_data: avc_codec_private_data(config),
                width: Some(width),
                height: Some(height),
                sampling_rate: None,
                channels: None,
            })
        }
        CodecConfig::Aac {
            esds,
            channel_count,
            sample_rate,
            ..
        } => {
            let codec_private_data = aac_codec_private_data(esds)?;
            let sampling_rate = AudioSpecificConfig::parse(&codec_private_data)
                .ok()
                .and_then(|asc| asc.effective_sampling_frequency())
                .unwrap_or(*sample_rate);
            Ok(SmoothCodec {
                stream_type: SmoothStreamType::Audio,
                fourcc: FOURCC_AACL,
                codec_private_data,
                width: None,
                height: None,
                sampling_rate: Some(sampling_rate),
                channels: Some(*channel_count),
            })
        }
        // A subtitle track re-muxed from an ISOBMFF `stpp` / `wvtt` sample entry
        // (issue #1186): the fragments carry the samples as they are, the
        // manifest names the sample format by its entry fourcc.
        CodecConfig::Subtitle {
            sample_entry: Some(entry),
            ..
        } => {
            let fourcc = match entry {
                crate::pipeline::SubtitleSampleEntry::Ttml(_) => FOURCC_STPP,
                crate::pipeline::SubtitleSampleEntry::WebVtt(_) => FOURCC_WVTT,
            };
            Ok(SmoothCodec {
                stream_type: SmoothStreamType::Text,
                fourcc,
                codec_private_data: Vec::new(),
                width: None,
                height: None,
                sampling_rate: None,
                channels: None,
            })
        }
        _ => Err(Error::InvalidInput(
            "Smooth Streaming supports only H.264 video, AAC-LC audio and ISOBMFF (stpp/wvtt) subtitles",
        )),
    }
}

// ---------------------------------------------------------------------------
// Segmentation + fragment building
// ---------------------------------------------------------------------------

/// The one decode-time origin a Smooth manifest's `c@t` values are relative
/// to: the earliest first-sample decode time across the presented tracks, on
/// one timescale (the same origin [`TimelineOrigin`](crate::media) gives the
/// other packagers, compared across timescales exactly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmoothOrigin {
    ticks: i64,
    timescale: Timescale,
}

/// One tick per second — the timescale a [`SmoothOrigin::default`] carries, so
/// an origin of `0` needs no specific track's timescale.
const DEFAULT_ORIGIN_TIMESCALE: Timescale = match Timescale::new(1) {
    Some(timescale) => timescale,
    None => Timescale::MICROS,
};

impl SmoothOrigin {
    /// An origin at `ticks` on `timescale`.
    pub const fn new(ticks: i64, timescale: Timescale) -> Self {
        Self { ticks, timescale }
    }

    /// The origin of `tracks`: the earliest first-sample `dts`, or `0` when no
    /// sample carries one. The bridge for a caller that still holds the
    /// samples; a live source whose stream starts at zero uses
    /// [`SmoothOrigin::default`].
    pub fn from_tracks<'a>(tracks: impl IntoIterator<Item = (Timescale, &'a [Sample])>) -> Self {
        let mut best: Option<(i64, Timescale)> = None;
        for (timescale, samples) in tracks {
            let Some(dts) = samples.iter().find_map(|s| s.timing.and_then(|t| t.dts())) else {
                continue;
            };
            let earlier = best.is_none_or(|(ticks, best_timescale)| {
                broadcast_core::timescale::cmp_ticks(dts, timescale, ticks, best_timescale).is_lt()
            });
            if earlier {
                best = Some((dts, timescale));
            }
        }
        match best {
            Some((ticks, timescale)) => Self { ticks, timescale },
            None => Self::default(),
        }
    }

    /// The origin expressed in `timescale` ticks, rounded down.
    fn scaled_to(&self, timescale: Timescale) -> i64 {
        rescale_i64(self.ticks, self.timescale, timescale, Rounding::Floor)
    }
}

impl Default for SmoothOrigin {
    fn default() -> Self {
        Self {
            ticks: 0,
            timescale: DEFAULT_ORIGIN_TIMESCALE,
        }
    }
}

/// One sample's timing in [`SMOOTH_TIMESCALE`] ticks.
#[derive(Debug, Clone, Copy)]
struct SmoothSampleTime {
    /// Decode duration: up to the next sample's decode time (so the `trun`
    /// durations of a fragment sum exactly to its `c@d`, with no per-sample
    /// rounding drift), the last sample's own duration.
    duration: u32,
    /// Composition offset (`pts - dts`).
    composition_offset: i32,
}

/// Rescale one fragment's samples to [`SMOOTH_TIMESCALE`]:
/// `next_start` is the smooth-tick start of the sample that *follows* the
/// fragment (the cut point), which ends the fragment's last sample; `None` for
/// the trailing fragment, whose last sample ends at its own duration.
fn smooth_times(
    samples: &[Sample],
    origin: SmoothOrigin,
    media_timescale: Timescale,
    next_start: Option<u64>,
) -> Result<Vec<SmoothSampleTime>> {
    let base = origin.scaled_to(media_timescale);
    let relative = |sample: &Sample| -> i64 {
        sample
            .timing
            .and_then(|t| t.dts())
            .map_or(0, |dts| dts.saturating_sub(base).max(0))
    };
    let starts: Vec<u64> = samples
        .iter()
        .map(|s| to_smooth_ticks(relative(s) as u64, media_timescale))
        .collect();
    samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let start = starts[i];
            let own_end = to_smooth_ticks(
                (relative(s) as u64)
                    .saturating_add(u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0))),
                media_timescale,
            );
            let end = match starts.get(i + 1).copied().or(next_start) {
                Some(next) if next >= start => next,
                _ => own_end,
            };
            let duration = u32::try_from(end - start).map_err(|_| {
                Error::InvalidValue(InvalidValue {
                    field: "trun sample_duration",
                    value: end - start,
                    reason: "exceeds 32 bits in the 10 MHz Smooth timescale",
                })
            })?;
            let cto = rescale_i64(
                i64::from(s.composition_offset()?.unwrap_or(0)),
                media_timescale,
                SMOOTH_DEFAULT_TIMESCALE,
                Rounding::Nearest,
            );
            let composition_offset = i32::try_from(cto).map_err(|_| {
                Error::InvalidValue(InvalidValue {
                    field: "trun sample_composition_time_offset",
                    value: cto.unsigned_abs(),
                    reason: "exceeds 32 bits in the 10 MHz Smooth timescale",
                })
            })?;
            Ok(SmoothSampleTime {
                duration,
                composition_offset,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// SmoothSegmenter
// ---------------------------------------------------------------------------

/// Per-track accumulation for the fragments being built.
#[derive(Debug)]
struct SmoothTrackState {
    spec: TrackSpec,
    /// Samples of the fragment being built, in decode order.
    pending: Vec<Sample>,
    /// Summed duration of `pending`, in this track's media timescale.
    pending_duration: u64,
    /// `mfhd.sequence_number` of this track's next fragment, 1-based and
    /// per-track (Smooth addresses a fragment by start time within a
    /// `StreamIndex`, so every stream counts its own).
    next_sequence: u32,
    /// Rounded-up target duration in this track's timescale.
    target_ticks: u64,
}

/// Cut a track stream into keyframe-aligned Smooth fragments, each a
/// `moof`+`tfxd`+`mdat` ([MS-SSTR] §2.2.4), emitted as
/// [`MuxOutput::Segment`].
///
/// The cut rule is the [`Segmenter`](crate::segmenter::Segmenter) anchor rule
/// applied per track (Smooth fragments are per-stream): a sync sample that
/// arrives once the track has buffered at least the target duration opens the
/// next fragment, so no sample is dropped or reordered and every fragment after
/// the first starts on a random-access point.
///
/// Every time the fragment carries — `c@t`/`c@d` in the summary, `tfxd`, and
/// the `trun` sample durations and composition offsets — is in
/// [`SMOOTH_TIMESCALE`]: a client builds the track's timescale from the
/// manifest (Smooth has no init segment), so media-timescale `trun` values
/// would play 111x/208x too short (issue #1022). `c@t` is relative to
/// `origin`, the decode time common to every track, so inter-track offsets
/// survive.
#[derive(Debug)]
pub struct SmoothSegmenter {
    tracks: Vec<SmoothTrackState>,
    origin: SmoothOrigin,
    /// Fragments finished but not yet polled, in emission order.
    ready: alloc::collections::VecDeque<MuxOutput>,
}

impl SmoothSegmenter {
    /// A segmenter for `tracks`, cutting each at `target_duration` on its own
    /// keyframes, with `c@t` measured from `origin`.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate
    /// `track_id`s, or `target_duration` is zero.
    pub fn new(
        tracks: Vec<TrackSpec>,
        origin: SmoothOrigin,
        target_duration: core::time::Duration,
    ) -> Result<Self> {
        if tracks.is_empty() {
            return Err(Error::InvalidInput(
                "smooth segmenter needs at least one track",
            ));
        }
        if target_duration.is_zero() {
            return Err(Error::InvalidInput("target_duration must be positive"));
        }
        for (i, a) in tracks.iter().enumerate() {
            if tracks[i + 1..].iter().any(|b| b.track_id == a.track_id) {
                return Err(Error::InvalidInput("duplicate track_id"));
            }
        }
        let seconds = target_duration.as_secs().max(1);
        Ok(Self {
            tracks: tracks
                .into_iter()
                .map(|spec| SmoothTrackState {
                    target_ticks: seconds.saturating_mul(u64::from(spec.timescale.get())),
                    spec,
                    pending: Vec::new(),
                    pending_duration: 0,
                    next_sequence: 1,
                })
                .collect(),
            origin,
            ready: alloc::collections::VecDeque::new(),
        })
    }

    /// Buffer one sample for `track`, in decode order, cutting the previous
    /// fragment first when this sample is a keyframe past the target.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] for an unannounced track, a fragment build
    /// failure, or [`MAX_PENDING_SAMPLES_PER_TRACK`] samples buffered without a
    /// keyframe to cut on (send [`MuxInput::Flush`] to close the trailing
    /// fragment).
    fn push_sample(&mut self, track: TrackId, sample: Sample) -> Result<()> {
        let index = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track)
            .ok_or(Error::InvalidInput("smooth: unknown track_id"))?;
        if sample.flags.is_sync
            && self.tracks[index].pending_duration >= self.tracks[index].target_ticks
            && !self.tracks[index].pending.is_empty()
        {
            let cut = self.relative_ticks(index, &sample);
            self.cut(
                index,
                Some(to_smooth_ticks(cut, self.tracks[index].spec.timescale)),
            )?;
        }
        if self.tracks[index].pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK {
            return Err(crate::segmenter::no_sync_sample_error());
        }
        let state = &mut self.tracks[index];
        state.pending_duration = state.pending_duration.saturating_add(u64::from(
            sample.timing.and_then(|t| t.duration()).unwrap_or(0),
        ));
        state.pending.push(sample);
        Ok(())
    }

    /// `sample`'s decode time relative to the origin, in its track's ticks.
    fn relative_ticks(&self, index: usize, sample: &Sample) -> u64 {
        let timescale = self.tracks[index].spec.timescale;
        let base = self.origin.scaled_to(timescale);
        sample
            .timing
            .and_then(|t| t.dts())
            .map_or(0, |dts| dts.saturating_sub(base).max(0)) as u64
    }

    /// Cut the buffered fragment of track `index`, whose end is `next_start`
    /// smooth ticks from the origin (`None`: its last sample's own end).
    fn cut(&mut self, index: usize, next_start: Option<u64>) -> Result<()> {
        let state = &mut self.tracks[index];
        if state.pending.is_empty() {
            return Ok(());
        }
        let timescale = state.spec.timescale;
        let base = self.origin.scaled_to(timescale);
        let start = state
            .pending
            .first()
            .and_then(|s| s.timing.and_then(|t| t.dts()))
            .map_or(0, |dts| dts.saturating_sub(base).max(0)) as u64;
        let times = smooth_times(&state.pending, self.origin, timescale, next_start)?;
        let start_smooth = to_smooth_ticks(start, timescale);
        let duration: u64 = times.iter().map(|t| u64::from(t.duration)).sum();
        let sequence = state.next_sequence;
        let data = build_smooth_fragment(
            state.spec.track_id,
            sequence,
            start_smooth,
            duration,
            &state.pending,
            &times,
        )?;
        let summary_samples = u32::try_from(state.pending.len()).unwrap_or(u32::MAX);
        let summary = SegmentSummary {
            track: state.spec.track_id,
            sequence: u64::from(sequence),
            start: start_smooth,
            duration,
            timescale: SMOOTH_DEFAULT_TIMESCALE.get(),
            bytes: state.pending.iter().map(|s| s.data.len() as u64).sum(),
            independent: state.pending.first().is_some_and(|s| s.flags.is_sync),
            discontinuous: false,
        };
        state.next_sequence += 1;
        state.pending.clear();
        state.pending_duration = 0;
        self.ready
            .push_back(MuxOutput::Segment(Box::new(SegmentOut {
                bytes: bytes::Bytes::from(data),
                summary,
                tracks: alloc::vec![crate::mux_stage::TrackPart {
                    track: state.spec.track_id,
                    start: start_smooth,
                    duration,
                    timescale: SMOOTH_DEFAULT_TIMESCALE.get(),
                    bytes: summary.bytes,
                    samples: summary_samples,
                }],
            })));
        Ok(())
    }
}

impl Stage for SmoothSegmenter {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => {
                if let Some(state) = self
                    .tracks
                    .iter_mut()
                    .find(|t| t.spec.track_id == spec.track_id)
                {
                    state.target_ticks = state.target_ticks.max(u64::from(spec.timescale.get()));
                    state.spec = *spec;
                }
                Ok(())
            }
            MuxInput::Sample { track, sample } => self.push_sample(track, sample),
            // A Smooth fragment states its own absolute start in `tfxd`, so a
            // decode-time jump needs no separate signalling.
            MuxInput::Discontinuity { .. } => Ok(()),
            MuxInput::Flush => {
                for index in 0..self.tracks.len() {
                    self.cut(index, None)?;
                }
                Ok(())
            }
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.feed(MuxInput::Flush, Timestamp::ZERO)
    }

    fn demand(&self) -> Demand {
        Demand::saturated_if(
            self.tracks
                .iter()
                .any(|t| t.pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK),
        )
    }
}

/// Build one self-contained Smooth fragment: `styp` + `moof`(`mfhd` `traf`(
/// `tfhd` `trun` `tfxd`)) + `mdat`.
///
/// `start_smooth` / `dur_smooth` are in [`SMOOTH_TIMESCALE`] ticks (for the
/// `tfxd`). Smooth fragments carry timing via the `tfxd` box, so no `tfdt` is
/// emitted (the fragment is addressed by its manifest start time).
fn build_smooth_fragment(
    track_id: TrackId,
    sequence_number: u32,
    start_smooth: u64,
    dur_smooth: u64,
    samples: &[Sample],
    times: &[SmoothSampleTime],
) -> Result<Vec<u8>> {
    let styp = SegmentTypeBox {
        major_brand: STYP_MAJOR_BRAND,
        minor_version: 0,
        compatible_brands: alloc::vec![STYP_MAJOR_BRAND, *b"msix"],
    };

    let (tfhd, trun) = media_fragment_run(
        isobmff::TrackId::new(track_id.value()),
        samples.iter().zip(times).map(|(s, t)| RunSample {
            duration: t.duration,
            size: s.data.len(),
            sync: s.flags.is_sync,
            composition_offset: t.composition_offset,
        }),
        None,
    )?;
    let tfxd = TfxdBox::new(start_smooth, dur_smooth);

    let mut moof = MovieFragmentBox::new(
        MovieFragmentHeaderBox::new(sequence_number),
        alloc::vec![TrackFragmentBox::new(tfhd, None, alloc::vec![trun])],
    );

    // The tfxd is a uuid box inside the traf, but MovieFragmentBox knows nothing
    // about it, so we assemble the fragment bytes manually: compute the moof
    // size *including* the tfxd, set the trun data_offset from that size, then
    // splice the tfxd into the serialized traf.
    let tfxd_len = tfxd.serialized_len();
    // moof size with tfxd added into the single traf.
    let moof_size = moof.serialized_len() + tfxd_len;
    let mdat_payload_len: u64 = samples.iter().map(|s| s.data.len() as u64).sum();
    let mdat_header =
        BoxHeader::for_payload(BoxType::from_bytes(MDAT), None, mdat_payload_len).header_size();
    let mdat_start = moof_size + mdat_header;
    let data_offset = i32::try_from(mdat_start).map_err(|_| {
        Error::InvalidValue(InvalidValue {
            field: "trun data_offset",
            value: mdat_start as u64,
            reason: "moof too large for the signed 32-bit trun data_offset",
        })
    })?;
    moof.traf[0].trun[0].data_offset = Some(data_offset);

    let base_moof = moof.try_to_bytes()?;
    let with_tfxd = splice_tfxd_into_traf(&base_moof, &tfxd)?;
    debug_assert_eq!(with_tfxd.len(), moof_size);

    let mut mdat_data = Vec::new();
    for s in samples {
        mdat_data.extend_from_slice(&s.data);
    }
    let mdat = MediaDataBox { data: mdat_data };

    let total = styp.serialized_len() + with_tfxd.len() + mdat.serialized_len();
    let mut out = alloc::vec![0u8; total];
    let mut c = 0usize;
    c += styp.serialize_into(&mut out[c..])?;
    out[c..c + with_tfxd.len()].copy_from_slice(&with_tfxd);
    c += with_tfxd.len();
    mdat.serialize_into(&mut out[c..])?;
    Ok(out)
}

/// Append a `tfxd` `uuid` box at the end of the (single) `traf` inside a
/// serialized `moof`, patching the `moof` and `traf` container box sizes.
///
/// The serialized `moof` layout is `moof`[`mfhd`, `traf`[`tfhd`, `trun`]]; the
/// `traf` is the last child of the `moof`, so appending the `tfxd` at the very
/// end of the buffer places it as the last child of that `traf`.
fn splice_tfxd_into_traf(moof: &[u8], tfxd: &TfxdBox) -> Result<Vec<u8>> {
    if moof.len() < BOX_HEADER_MIN_SIZE || !starts_with_moof(&moof[BOX_TYPE_OFFSET..]) {
        return Err(Error::UnexpectedBox { expected: "moof" });
    }
    // Locate the traf: walk moof children, it is the box after mfhd.
    let mfhd_size = be_u32(moof, BOX_HEADER_MIN_SIZE, "moof box header")? as usize;
    let traf_off = BOX_HEADER_MIN_SIZE + mfhd_size;
    if moof.len() < traf_off + BOX_HEADER_MIN_SIZE
        || &moof[traf_off + BOX_TYPE_OFFSET..traf_off + BOX_HEADER_MIN_SIZE] != TRAF.as_slice()
    {
        return Err(Error::UnexpectedBox { expected: "traf" });
    }
    let old_traf_size = be_u32(moof, traf_off, "traf box header")? as usize;

    let tfxd_bytes = tfxd.try_to_bytes()?;
    let mut out = Vec::with_capacity(moof.len() + tfxd_bytes.len());
    out.extend_from_slice(moof);
    // The traf is the last child (mfhd then traf), so the tfxd goes at the end.
    out.extend_from_slice(&tfxd_bytes);

    // Patch moof and traf sizes; both were written in the compact 32-bit form,
    // which has no room for a `largesize` (§4.2), so a size past u32 is refused.
    let too_big = |size: usize| {
        Error::InvalidValue(InvalidValue {
            field: "moof/traf box size",
            value: size as u64,
            reason: "exceeds the 32-bit box size of the serialized moof",
        })
    };
    let new_moof_size = u32::try_from(out.len()).map_err(|_| too_big(out.len()))?;
    out[0..4].copy_from_slice(&new_moof_size.to_be_bytes());
    let traf_size = old_traf_size + tfxd_bytes.len();
    let new_traf_size = u32::try_from(traf_size).map_err(|_| too_big(traf_size))?;
    out[traf_off..traf_off + core::mem::size_of::<u32>()]
        .copy_from_slice(&new_traf_size.to_be_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("a non-zero timescale")
    }

    use super::*;
    use crate::collect::CollectedTrack as Track;
    use crate::ir::DataIdentity;
    use broadcast_core::timescale::Timescale;

    #[test]
    fn tfxd_round_trip() {
        let t = TfxdBox::new(0x0011_2233_4455_6677, 0x0089_ABCD);
        let bytes = t.try_to_bytes().unwrap();
        // 8 (box hdr) + 16 (usertype) + 4 (fullbox) + 16 (payload) = 44.
        assert_eq!(bytes.len(), 44);
        assert_eq!(&bytes[4..8], b"uuid");
        assert_eq!(&bytes[8..24], &TFXD_UUID);
        // body = everything after box header(8) + usertype(16) = offset 24.
        let p = TfxdBox::parse_body(&bytes[24..]).unwrap();
        assert_eq!(p, t);
        assert_eq!(p.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn to_smooth_ticks_scales() {
        // 1 second at media timescale 90000 → 10_000_000 smooth ticks.
        assert_eq!(to_smooth_ticks(90_000, timescale(90_000)), 10_000_000);
    }

    fn data_spec(track_id: u32, rate: u32) -> crate::pipeline::TrackSpec {
        crate::pipeline::TrackSpec::new(
            TrackId::new(track_id),
            timescale(rate),
            CodecConfig::Data {
                identity: DataIdentity::Ts {
                    stream_type: 0x86,
                    descriptors: alloc::vec![],
                    carriage: crate::pipeline::DataCarriage::Sections,
                },
            },
        )
    }

    fn timed_track(track_id: u32, rate: u32, samples: &[(i64, u32)]) -> Track {
        let samples = samples
            .iter()
            .map(|&(dts, duration)| {
                Sample::new(
                    alloc::vec![0u8; 4],
                    Some(dts),
                    Some(dts),
                    Some(duration),
                    true,
                )
                .expect("valid timing")
            })
            .collect();
        Track::new(data_spec(track_id, rate), samples)
    }

    #[test]
    fn tfxd_serialize_reports_the_full_box_length() {
        let t = TfxdBox::new(0x0011_2233_4455_6677, 0x0089_ABCD);
        let mut buf = alloc::vec![0u8; t.serialized_len()];
        let n = t.serialize_into(&mut buf).unwrap();
        assert_eq!(n, t.serialized_len(), "returned count must match the box");
        assert_eq!(n, 44, "8 hdr + 16 usertype + 4 fullbox + 16 times");
    }

    #[test]
    fn splice_tfxd_rejects_a_non_moof_buffer() {
        let tfxd = TfxdBox::new(1, 2);
        let mdat = MediaDataBox {
            data: alloc::vec![0u8; 4],
        }
        .try_to_bytes()
        .unwrap();
        assert!(matches!(
            splice_tfxd_into_traf(&mdat, &tfxd),
            Err(Error::UnexpectedBox { expected: "moof" })
        ));
    }

    #[test]
    fn quality_level_audio_attributes_only_on_audio_streams() {
        let video = StreamInfo {
            stream_type: SmoothStreamType::Video,
            fourcc: FOURCC_H264,
            codec_private_data: alloc::vec![1, 2],
            bitrate: 1_000_000,
            width: Some(640),
            height: Some(360),
            sampling_rate: Some(48_000),
            channels: Some(2),
            qualities: 1,
            fragments: alloc::vec![],
            total_duration: 0,
        };
        let level = quality_level(&video);
        assert_eq!(level.width, Some(640));
        assert_eq!(level.height, Some(360));
        assert_eq!(level.sampling_rate, None, "video carries no SamplingRate");
        assert_eq!(level.channels, None, "video carries no Channels");
        assert_eq!(level.bits_per_sample, None);
        assert_eq!(level.audio_tag, None);
    }

    #[test]
    fn smooth_timeline_uses_the_gap_to_the_next_sample() {
        // 90 kHz, dts steps 3003 but declared durations 3000: a plain
        // duration sum would drift from the timeline the manifest describes.
        let track = timed_track(1, 90_000, &[(0, 3000), (3003, 3000), (6006, 3000)]);
        let origin = SmoothOrigin::from_tracks([(track.spec.timescale, track.samples.as_slice())]);
        let times =
            smooth_times(&track.samples, origin, timescale(90_000), None).expect("timeline");
        let durations: Vec<u32> = times.iter().map(|t| t.duration).collect();
        assert_eq!(durations, alloc::vec![333_667, 333_666, 333_334]);
        let sum: u64 = times.iter().map(|t| u64::from(t.duration)).sum();
        assert_eq!(sum, 1_000_667, "trun durations sum to the timeline span");
    }

    #[test]
    fn smooth_timeline_falls_back_to_own_duration_when_dts_goes_backwards() {
        let origin_track = timed_track(2, 1000, &[(0, 1000)]);
        let track = timed_track(1, 1000, &[(1000, 1000), (500, 1000)]);
        let origin = SmoothOrigin::from_tracks([
            (origin_track.spec.timescale, origin_track.samples.as_slice()),
            (track.spec.timescale, track.samples.as_slice()),
        ]);
        let times = smooth_times(&track.samples, origin, timescale(1000), None).expect("timeline");
        let durations: Vec<u32> = times.iter().map(|t| t.duration).collect();
        assert_eq!(durations, alloc::vec![10_000_000, 10_000_000]);
    }

    #[test]
    fn smooth_timeline_reports_the_true_over_range_duration() {
        let origin_track = timed_track(2, 1000, &[(0, 1000)]);
        // 500 s at 1 kHz = 5e9 ticks in the 10 MHz Smooth timescale (> u32::MAX).
        let track = timed_track(1, 1000, &[(1000, 500_000)]);
        let origin = SmoothOrigin::from_tracks([
            (origin_track.spec.timescale, origin_track.samples.as_slice()),
            (track.spec.timescale, track.samples.as_slice()),
        ]);
        match smooth_times(&track.samples, origin, timescale(1000), None) {
            Err(Error::InvalidValue(v)) => {
                assert_eq!(v.field, "trun sample_duration");
                assert_eq!(v.value, 5_000_000_000);
            }
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn splice_tfxd_reads_the_whole_32_bit_traf_size_above_64_kib() {
        // 6000 samples at 12 bytes each: the traf is 72 032 bytes, so its size
        // needs the second byte (`0x01` in 0x0001_1960).
        const COUNT: usize = 6000;
        let samples: Vec<Sample> = (0..COUNT)
            .map(|i| {
                Sample::new(alloc::vec![0xAB; 4], None, None, Some(1), i == 0)
                    .expect("valid timing")
            })
            .collect();
        let times: Vec<SmoothSampleTime> = (0..COUNT)
            .map(|_| SmoothSampleTime {
                duration: 10_000,
                composition_offset: 0,
            })
            .collect();
        let frag = build_smooth_fragment(
            TrackId::new(1),
            1,
            0,
            COUNT as u64 * 10_000,
            &samples,
            &times,
        )
        .expect("build fragment");

        let styp_len = SegmentTypeBox {
            major_brand: STYP_MAJOR_BRAND,
            minor_version: 0,
            compatible_brands: alloc::vec![STYP_MAJOR_BRAND, *b"msix"],
        }
        .serialized_len();
        let moof_size =
            u32::from_be_bytes(frag[styp_len..styp_len + 4].try_into().expect("moof size"))
                as usize;
        MovieFragmentBox::parse_body(&frag[styp_len + 8..styp_len + moof_size])
            .expect("patched moof parses");
        let mfhd_size = u32::from_be_bytes(
            frag[styp_len + 8..styp_len + 12]
                .try_into()
                .expect("mfhd size"),
        ) as usize;
        let traf_off = styp_len + 8 + mfhd_size;
        let traf_size =
            u32::from_be_bytes(frag[traf_off..traf_off + 4].try_into().expect("traf size"))
                as usize;
        assert_eq!(
            traf_size,
            moof_size - 8 - mfhd_size,
            "the traf is the last moof child and spans the rest of it"
        );
        assert!(
            traf_size > 65_536,
            "the fixture must exceed 64 KiB: {traf_size}"
        );
        assert_eq!(
            crate::smooth_fragment::fragment_tfxd(&frag),
            Some(TfxdBox::new(0, COUNT as u64 * 10_000))
        );
    }
}
