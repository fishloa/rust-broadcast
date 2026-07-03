//! Microsoft Smooth Streaming (ISM/PIFF) output — [MS-SSTR].
//!
//! The Smooth-Streaming sibling of [`crate::dash::DashPackager`] and
//! [`crate::media::HlsPackager`]: [`SmoothPackager`] renders a [`Media`] as a
//! Smooth **client Manifest** (XML) plus a set of Smooth **fragment** responses,
//! each a fragmented-MP4 `moof`+`mdat` carrying the Smooth-specific `tfxd`
//! `uuid` box.
//!
//! # Structure ([MS-SSTR]; see `transmux/docs/smooth/ms-sstr.md`)
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
//! (ISO/IEC 14496-12:2015 §8.8) the crate already builds; this module reuses
//! [`crate::segmenter::Segmenter`] for keyframe-aligned segmentation and injects
//! the `tfxd` `uuid` box into each fragment's `traf`.
//!
//! FourCC / `CodecPrivateData` (§2.2.2.5):
//! - Video `FourCC="H264"`: `CodecPrivateData` = the hex of SPS+PPS as
//!   start-code-prefixed NAL units (`00000001 <sps> 00000001 <pps>`).
//! - Audio `FourCC="AACL"`: `CodecPrivateData` = the hex of the
//!   AudioSpecificConfig; `AudioTag="255"` (raw AAC).
//!
//! Like the DASH / HLS packagers, the manifest is emitted with a tiny
//! hand-rolled XML writer and the crate stays dependency-free. Integer
//! arithmetic only (`no_std` + `alloc`).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::aac_asc::AudioSpecificConfig;
use crate::box_types::{BoxHeader, BoxType, UUID_TYPE_SIZE};
use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::movie_fragment::{
    MovieFragmentBox, MovieFragmentHeaderBox, TFHD_DEFAULT_BASE_IS_MOOF, TRUN_DATA_OFFSET_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, TrackFragmentBox, TrackFragmentHeaderBox,
    TrackFragmentRunBox, TrunSample,
};
use crate::pipeline::{CodecConfig, Sample};
use crate::segments::{MediaDataBox, SegmentTypeBox};

/// The Smooth Streaming default manifest time scale — 10 MHz (100 ns ticks),
/// [MS-SSTR] §2.2.2 (`SmoothStreamingMedia@TimeScale`).
pub const SMOOTH_TIMESCALE: u64 = 10_000_000;

/// `SmoothStreamingMedia@MajorVersion` for the manifest this module emits.
const MAJOR_VERSION: u32 = 2;
/// `SmoothStreamingMedia@MinorVersion`.
const MINOR_VERSION: u32 = 0;

/// The `tfxd` `uuid` extended-type — [MS-SSTR] §2.2.4.4
/// (`6d1d9b05-42d5-44e6-80e2-141daff757b2`).
pub const TFXD_UUID: [u8; UUID_TYPE_SIZE] = [
    0x6d, 0x1d, 0x9b, 0x05, 0x42, 0xd5, 0x44, 0xe6, 0x80, 0xe2, 0x14, 0x1d, 0xaf, 0xf7, 0x57, 0xb2,
];

/// Video FourCC in the Smooth `QualityLevel` (§2.2.2.5) — H.264/AVC.
pub const FOURCC_H264: &str = "H264";
/// Audio FourCC in the Smooth `QualityLevel` (§2.2.2.5) — AAC-LC.
pub const FOURCC_AACL: &str = "AACL";

/// `AudioTag` for raw AAC in the Smooth `QualityLevel` (§2.2.2.5).
const AUDIO_TAG_AAC: u32 = 255;
/// Bits-per-sample advertised for AAC audio.
const AAC_BITS_PER_SAMPLE: u16 = 16;

/// Annex B start code prefixing each parameter-set NAL in `CodecPrivateData`.
const START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// The `styp` brand emitted ahead of a self-contained Smooth fragment (kept
/// consistent with the crate's CMAF media segments).
const STYP_MAJOR_BRAND: [u8; 4] = *b"msdh";

// --- sample_flags (ISO/IEC 14496-12:2015 §8.8.3.1) --------------------------
/// Sample flags for a sync sample (I-frame).
const SAMPLE_FLAGS_SYNC: u32 = 0x0200_0000;
/// Sample flags for a non-sync sample.
const SAMPLE_FLAGS_NON_SYNC: u32 = 0x0101_0000;

/// The media kind of a Smooth `StreamIndex`.
///
/// Determines the `Type` attribute, the FourCC, and which
/// geometry / audio attributes the `QualityLevel` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmoothStreamType {
    /// Video stream — `Type="video"`, `FourCC="H264"`.
    Video,
    /// Audio stream — `Type="audio"`, `FourCC="AACL"`.
    Audio,
}

impl SmoothStreamType {
    /// The [MS-SSTR] `StreamIndex@Type` token (`"video"` / `"audio"`).
    pub fn name(&self) -> &'static str {
        match self {
            SmoothStreamType::Video => "video",
            SmoothStreamType::Audio => "audio",
        }
    }
}

broadcast_common::impl_spec_display!(SmoothStreamType);

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
        let need = 4 + Self::PAYLOAD_LEN;
        if body.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: body.len(),
                what: "tfxd body",
            });
        }
        let p = &body[4..];
        let fragment_absolute_time =
            u64::from_be_bytes([p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]]);
        let fragment_duration =
            u64::from_be_bytes([p[8], p[9], p[10], p[11], p[12], p[13], p[14], p[15]]);
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
        let hdr = BoxHeader::new(0, BoxType::from_bytes(*b"uuid"), Some(TFXD_UUID));
        hdr.serialized_len() + 4 + Self::PAYLOAD_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        // uuid box header (size, "uuid", usertype = TFXD_UUID).
        let hdr = BoxHeader::new(need as u64, BoxType::from_bytes(*b"uuid"), Some(TFXD_UUID));
        let mut c = hdr.serialize_into(buf)?;
        // FullBox header: version + 24-bit flags(0).
        buf[c] = Self::VERSION;
        buf[c + 1] = 0;
        buf[c + 2] = 0;
        buf[c + 3] = 0;
        c += 4;
        buf[c..c + 8].copy_from_slice(&self.fragment_absolute_time.to_be_bytes());
        c += 8;
        buf[c..c + 8].copy_from_slice(&self.fragment_duration.to_be_bytes());
        c += 8;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SmoothPackager
// ---------------------------------------------------------------------------

/// One Smooth fragment: its packaged `moof`+`mdat` bytes plus the timing needed
/// for the manifest `c` element.
#[derive(Debug, Clone)]
pub struct SmoothFragment {
    /// Track ID this fragment belongs to.
    pub track_id: u32,
    /// `mfhd.sequence_number` (1-based, monotonically increasing).
    pub sequence_number: u32,
    /// Fragment start time, in [`SMOOTH_TIMESCALE`] ticks (the `c@t` on the first).
    pub start_time: u64,
    /// Fragment duration, in [`SMOOTH_TIMESCALE`] ticks (the `c@d`).
    pub duration: u64,
    /// The self-contained fragment bytes: `styp` + `moof`(+`tfxd`) + `mdat`.
    pub data: Vec<u8>,
}

/// The Smooth Streaming output: the client Manifest plus every fragment.
#[derive(Debug, Clone)]
pub struct SmoothOutput {
    /// The Smooth client Manifest XML.
    pub manifest: String,
    /// Every Smooth fragment, in emission order (grouped per track).
    pub fragments: Vec<SmoothFragment>,
}

/// Render a [`Media`] as Microsoft Smooth Streaming ([MS-SSTR]).
///
/// Segments each track keyframe-aligned (via [`crate::segmenter::Segmenter`]),
/// emits one Smooth fragment (`moof`+`tfxd`+`mdat`) per segment, and builds a
/// client Manifest whose `c` timeline mirrors the emitted fragments. VOD only
/// (no live `tfrf` look-ahead).
#[derive(Debug, Clone)]
pub struct SmoothPackager {
    /// Target fragment duration, in whole seconds (keyframe-aligned cut target).
    pub target_duration_secs: u32,
}

impl Default for SmoothPackager {
    fn default() -> Self {
        Self {
            target_duration_secs: 2,
        }
    }
}

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
    /// Convert a tick count in the track's media timescale to
    /// [`SMOOTH_TIMESCALE`] ticks with integer round-to-nearest (`no_std`-safe).
    fn to_smooth_ticks(ticks: u64, media_timescale: u32) -> u64 {
        let ts = media_timescale.max(1) as u64;
        // round(ticks * SMOOTH_TIMESCALE / ts)
        let num = ticks.saturating_mul(SMOOTH_TIMESCALE);
        (num + ts / 2) / ts
    }

    /// The `CodecPrivateData` bytes for a video track: start-code-prefixed
    /// SPS+PPS NAL units (§2.2.2.5).
    fn video_codec_private_data(config: &crate::avc_config::AVCConfigurationBox) -> Vec<u8> {
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

    /// The `CodecPrivateData` bytes for an AAC track: the raw
    /// AudioSpecificConfig carried in the `esds` (§2.2.2.5).
    fn audio_codec_private_data(esds: &crate::mp4esds::EsdsBox) -> Result<Vec<u8>> {
        let dsi = esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|dc| dc.decoder_specific_info.as_ref())
            .ok_or(Error::UnexpectedBox {
                expected: "DecoderSpecificInfo (AudioSpecificConfig) in esds",
            })?;
        Ok(dsi.data.clone())
    }

    /// Segment one track into Smooth fragments and resolve its manifest info.
    ///
    /// Cuts the track keyframe-aligned into fragments; for each, builds the
    /// `moof`+`tfxd`+`mdat` bytes and records the `c` timing.
    fn build_track(
        &self,
        track: &Track,
        next_seq: &mut u32,
        fragments_out: &mut Vec<SmoothFragment>,
    ) -> Result<StreamInfo> {
        let media_timescale = track.spec.timescale.max(1);
        let (stream_type, fourcc, codec_private_data, width, height, sampling_rate, channels) =
            resolve_codec(&track.spec.config)?;

        // Segment keyframe-aligned into groups of samples.
        let groups = segment_samples(&track.samples, media_timescale, self.target_duration_secs);

        let mut fragment_timings = Vec::with_capacity(groups.len());
        let mut media_decode_time = 0u64; // in the media timescale
        let mut total_bytes = 0u64;
        let mut total_media_ticks = 0u64;

        for group in &groups {
            let group_media_dur: u64 = group.iter().map(|s| s.duration as u64).sum();
            let start_smooth = Self::to_smooth_ticks(media_decode_time, media_timescale);
            let dur_smooth =
                Self::to_smooth_ticks(media_decode_time + group_media_dur, media_timescale)
                    - start_smooth;

            let data = build_smooth_fragment(
                track.spec.track_id,
                *next_seq,
                start_smooth,
                dur_smooth,
                group,
            )?;

            for s in group.iter() {
                total_bytes += s.data.len() as u64;
            }
            fragments_out.push(SmoothFragment {
                track_id: track.spec.track_id,
                sequence_number: *next_seq,
                start_time: start_smooth,
                duration: dur_smooth,
                data,
            });
            fragment_timings.push(FragmentTiming {
                start_time: start_smooth,
                duration: dur_smooth,
            });
            *next_seq += 1;
            media_decode_time += group_media_dur;
            total_media_ticks += group_media_dur;
        }

        let total_duration: u64 = fragment_timings.iter().map(|f| f.duration).sum();

        // bitrate = total coded bits / duration in seconds (integer round);
        // checked_div guards the zero-duration case (→ 0).
        let bitrate = {
            let bits_ts = total_bytes
                .saturating_mul(8)
                .saturating_mul(media_timescale as u64);
            (bits_ts + total_media_ticks / 2)
                .checked_div(total_media_ticks)
                .unwrap_or(0)
        }
        .max(1);

        Ok(StreamInfo {
            stream_type,
            fourcc,
            codec_private_data,
            bitrate,
            width,
            height,
            sampling_rate,
            channels,
            fragments: fragment_timings,
            total_duration,
        })
    }

    /// Render the Smooth client Manifest XML for the resolved streams.
    fn render_manifest(&self, streams: &[StreamInfo]) -> String {
        let mut w = XmlWriter::new();
        w.declaration();

        let duration = streams.iter().map(|s| s.total_duration).max().unwrap_or(0);
        w.open(
            "SmoothStreamingMedia",
            &[
                ("MajorVersion", MAJOR_VERSION.to_string()),
                ("MinorVersion", MINOR_VERSION.to_string()),
                ("Duration", duration.to_string()),
                ("TimeScale", SMOOTH_TIMESCALE.to_string()),
            ],
        );

        for s in streams {
            let ty = s.stream_type.name();
            let url = format!("QualityLevels({{bitrate}})/Fragments({ty}={{start time}})");
            w.open(
                "StreamIndex",
                &[
                    ("Type", ty.to_string()),
                    ("Subtype", String::new()),
                    ("Chunks", s.fragments.len().to_string()),
                    ("QualityLevels", "1".to_string()),
                    ("Url", url),
                ],
            );

            // QualityLevel
            let mut ql = alloc::vec![
                ("Index", "0".to_string()),
                ("Bitrate", s.bitrate.to_string()),
                ("FourCC", s.fourcc.to_string()),
            ];
            match s.stream_type {
                SmoothStreamType::Video => {
                    if let (Some(wd), Some(ht)) = (s.width, s.height) {
                        ql.push(("MaxWidth", wd.to_string()));
                        ql.push(("MaxHeight", ht.to_string()));
                    }
                    ql.push(("CodecPrivateData", hex_upper(&s.codec_private_data)));
                }
                SmoothStreamType::Audio => {
                    if let Some(sr) = s.sampling_rate {
                        ql.push(("SamplingRate", sr.to_string()));
                    }
                    if let Some(ch) = s.channels {
                        ql.push(("Channels", ch.to_string()));
                    }
                    ql.push(("BitsPerSample", AAC_BITS_PER_SAMPLE.to_string()));
                    ql.push(("AudioTag", AUDIO_TAG_AAC.to_string()));
                    ql.push(("CodecPrivateData", hex_upper(&s.codec_private_data)));
                }
            }
            w.empty("QualityLevel", &ql);

            // One `c` per fragment: `d` always; `t` on the first only; `n` ordinal.
            for (i, f) in s.fragments.iter().enumerate() {
                let mut c = alloc::vec![("n", i.to_string())];
                if i == 0 {
                    c.push(("t", f.start_time.to_string()));
                }
                c.push(("d", f.duration.to_string()));
                w.empty("c", &c);
            }

            w.close("StreamIndex");
        }

        w.close("SmoothStreamingMedia");
        w.finish()
    }
}

impl broadcast_common::Package for SmoothPackager {
    type Media = Media;
    type Output = SmoothOutput;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<SmoothOutput> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }
        let mut fragments = Vec::new();
        let mut streams = Vec::with_capacity(media.tracks.len());
        // Per-track sequence numbering, 1-based (each track's fragments count
        // up independently, as Smooth fragment responses are addressed by
        // start time within a StreamIndex).
        for track in &media.tracks {
            let mut next_seq = 1u32;
            streams.push(self.build_track(track, &mut next_seq, &mut fragments)?);
        }
        let manifest = self.render_manifest(&streams);
        Ok(SmoothOutput {
            manifest,
            fragments,
        })
    }
}

// ---------------------------------------------------------------------------
// Codec resolution
// ---------------------------------------------------------------------------

type ResolvedCodec = (
    SmoothStreamType,
    &'static str,
    Vec<u8>,
    Option<u32>,
    Option<u32>,
    Option<u32>,
    Option<u16>,
);

/// Resolve the Smooth stream type, FourCC, `CodecPrivateData`, and per-kind
/// parameters for a track's [`CodecConfig`].
///
/// Only the [MS-SSTR]-defined H.264 (video) and AAC-LC (audio) codecs are
/// supported; other codecs return [`Error::InvalidInput`].
fn resolve_codec(config: &CodecConfig) -> Result<ResolvedCodec> {
    match config {
        CodecConfig::Avc { config, .. } => {
            let cpd = SmoothPackager::video_codec_private_data(config);
            // Prefer SPS-decoded geometry (authoritative coded dimensions).
            let (w, h) = config
                .config
                .sps
                .first()
                .and_then(|sps| sps.decode().ok())
                .map(|i| (i.width, i.height))
                .ok_or(Error::InvalidInput(
                    "AVC track has no decodable SPS for Smooth geometry",
                ))?;
            Ok((
                SmoothStreamType::Video,
                FOURCC_H264,
                cpd,
                Some(w),
                Some(h),
                None,
                None,
            ))
        }
        CodecConfig::Aac {
            esds,
            channel_count,
            sample_rate,
            ..
        } => {
            let cpd = SmoothPackager::audio_codec_private_data(esds)?;
            // Prefer the ASC-decoded sampling rate; fall back to the entry.
            let rate = AudioSpecificConfig::parse(&cpd)
                .ok()
                .and_then(asc_sampling_rate)
                .unwrap_or(*sample_rate);
            Ok((
                SmoothStreamType::Audio,
                FOURCC_AACL,
                cpd,
                None,
                None,
                Some(rate),
                Some(*channel_count),
            ))
        }
        _ => Err(Error::InvalidInput(
            "Smooth Streaming supports only H.264 video and AAC-LC audio",
        )),
    }
}

/// The effective sampling rate from a decoded ASC (explicit rate if present,
/// else the rate for the `samplingFrequencyIndex`, ISO/IEC 14496-3 Table 1.10).
fn asc_sampling_rate(asc: AudioSpecificConfig) -> Option<u32> {
    if let Some(fs) = asc.sampling_frequency {
        return Some(fs);
    }
    const RATES: [u32; 13] = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
    ];
    RATES
        .get(asc.sampling_frequency_index.raw() as usize)
        .copied()
}

// ---------------------------------------------------------------------------
// Segmentation + fragment building
// ---------------------------------------------------------------------------

/// Split a track's samples into keyframe-aligned groups, cutting a new group on
/// a sync sample once the current group has reached the target duration.
///
/// Mirrors the [`Segmenter`](crate::segmenter::Segmenter) anchor-cut policy
/// applied to a single track: every group after the first begins on a
/// random-access point, no sample is dropped or reordered.
fn segment_samples(samples: &[Sample], media_timescale: u32, target_secs: u32) -> Vec<&[Sample]> {
    let mut groups = Vec::new();
    if samples.is_empty() {
        return groups;
    }
    let target_ticks = (target_secs.max(1) as u64).saturating_mul(media_timescale.max(1) as u64);

    let mut start = 0usize;
    let mut acc_dur = 0u64;
    for (i, s) in samples.iter().enumerate() {
        // Cut before this sample when it is a keyframe past the target.
        if i > start && s.is_sync && acc_dur >= target_ticks {
            groups.push(&samples[start..i]);
            start = i;
            acc_dur = 0;
        }
        acc_dur += s.duration as u64;
    }
    groups.push(&samples[start..]);
    groups
}

/// Build one self-contained Smooth fragment: `styp` + `moof`(`mfhd` `traf`(
/// `tfhd` `trun` `tfxd`)) + `mdat`.
///
/// `start_smooth` / `dur_smooth` are in [`SMOOTH_TIMESCALE`] ticks (for the
/// `tfxd`). Smooth fragments carry timing via the `tfxd` box, so no `tfdt` is
/// emitted (the fragment is addressed by its manifest start time).
fn build_smooth_fragment(
    track_id: u32,
    sequence_number: u32,
    start_smooth: u64,
    dur_smooth: u64,
    samples: &[Sample],
) -> Result<Vec<u8>> {
    let styp = SegmentTypeBox {
        major_brand: STYP_MAJOR_BRAND,
        minor_version: 0,
        compatible_brands: alloc::vec![STYP_MAJOR_BRAND, *b"msix"],
    };

    let any_cts = samples.iter().any(|s| s.composition_offset != 0);
    let trun_samples: Vec<TrunSample> = samples
        .iter()
        .map(|s| TrunSample {
            sample_duration: Some(s.duration),
            sample_size: Some(s.data.len() as u32),
            sample_flags: Some(if s.is_sync {
                SAMPLE_FLAGS_SYNC
            } else {
                SAMPLE_FLAGS_NON_SYNC
            }),
            sample_composition_time_offset: if any_cts {
                Some(s.composition_offset)
            } else {
                None
            },
        })
        .collect();

    let mut tr_flags = TRUN_DATA_OFFSET_PRESENT
        | TRUN_SAMPLE_DURATION_PRESENT
        | TRUN_SAMPLE_SIZE_PRESENT
        | TRUN_SAMPLE_FLAGS_PRESENT;
    let version = if any_cts {
        tr_flags |= TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT;
        1u8
    } else {
        0u8
    };

    let trun = TrackFragmentRunBox {
        version,
        tr_flags,
        data_offset: Some(0),
        first_sample_flags: None,
        samples: trun_samples,
    };
    let tfhd = TrackFragmentHeaderBox {
        flags: TFHD_DEFAULT_BASE_IS_MOOF,
        track_id,
        base_data_offset: None,
        sample_description_index: None,
        default_sample_duration: None,
        default_sample_size: None,
        default_sample_flags: None,
    };
    let tfxd = TfxdBox::new(start_smooth, dur_smooth);

    let mut moof = MovieFragmentBox {
        mfhd: MovieFragmentHeaderBox::new(sequence_number),
        traf: alloc::vec![TrackFragmentBox {
            tfhd,
            tfdt: None,
            trun: alloc::vec![trun],
        }],
    };

    // The tfxd is a uuid box inside the traf, but MovieFragmentBox knows nothing
    // about it, so we assemble the fragment bytes manually: compute the moof
    // size *including* the tfxd, set the trun data_offset from that size, then
    // splice the tfxd into the serialized traf.
    let tfxd_len = tfxd.serialized_len();
    // moof size with tfxd added into the single traf.
    let moof_size = moof.serialized_len() + tfxd_len;
    let mdat_start = moof_size + 8; // +8 for the mdat box header
    moof.traf[0].trun[0].data_offset = Some(mdat_start as i32);

    // Serialize the moof (without tfxd), then splice the tfxd in at the end of
    // the traf and patch the two container sizes (moof, traf).
    let base_moof = moof.to_bytes();
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
    if moof.len() < 8 || &moof[4..8] != b"moof" {
        return Err(Error::UnexpectedBox { expected: "moof" });
    }
    // Locate the traf: walk moof children, it is the box after mfhd.
    let mfhd_size = u32::from_be_bytes([moof[8], moof[9], moof[10], moof[11]]) as usize;
    let traf_off = 8 + mfhd_size;
    if moof.len() < traf_off + 8 || &moof[traf_off + 4..traf_off + 8] != b"traf" {
        return Err(Error::UnexpectedBox { expected: "traf" });
    }
    let old_traf_size = u32::from_be_bytes([
        moof[traf_off],
        moof[traf_off + 1],
        moof[traf_off + 2],
        moof[traf_off + 3],
    ]) as usize;

    let tfxd_bytes = tfxd.to_bytes();
    let mut out = Vec::with_capacity(moof.len() + tfxd_bytes.len());
    out.extend_from_slice(moof);
    // The traf is the last child (mfhd then traf), so the tfxd goes at the end.
    out.extend_from_slice(&tfxd_bytes);

    // Patch moof size (bytes [0..4]).
    let new_moof_size = out.len() as u32;
    out[0..4].copy_from_slice(&new_moof_size.to_be_bytes());
    // Patch traf size.
    let new_traf_size = (old_traf_size + tfxd_bytes.len()) as u32;
    out[traf_off..traf_off + 4].copy_from_slice(&new_traf_size.to_be_bytes());
    Ok(out)
}

/// Hex-encode bytes as uppercase (no separators), for `CodecPrivateData`.
fn hex_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0F) as usize] as char);
    }
    s
}

// ---------------------------------------------------------------------------
// Tiny XML writer — no external dependency (dep-free like DashPackager).
// ---------------------------------------------------------------------------

/// A minimal, indentation-aware XML element writer (see the DASH sibling).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tfxd_round_trip() {
        let t = TfxdBox::new(0x0011_2233_4455_6677, 0x0089_ABCD);
        let bytes = t.to_bytes();
        // 8 (box hdr) + 16 (usertype) + 4 (fullbox) + 16 (payload) = 44.
        assert_eq!(bytes.len(), 44);
        assert_eq!(&bytes[4..8], b"uuid");
        assert_eq!(&bytes[8..24], &TFXD_UUID);
        // body = everything after box header(8) + usertype(16) = offset 24.
        let p = TfxdBox::parse_body(&bytes[24..]).unwrap();
        assert_eq!(p, t);
        assert_eq!(p.to_bytes(), bytes);
    }

    #[test]
    fn hex_upper_encodes() {
        assert_eq!(hex_upper(&[0x00, 0x01, 0xAB, 0xFF]), "0001ABFF");
    }

    #[test]
    fn segment_samples_keyframe_aligned() {
        // 4 samples, timescale 1 tick/sample, target 2s: sync at 0 and 2.
        let mk = |sync: bool| Sample {
            data: alloc::vec![0u8; 4],
            duration: 1,
            is_sync: sync,
            composition_offset: 0,
            source_timing: None,
        };
        let samples = alloc::vec![mk(true), mk(false), mk(true), mk(false)];
        let groups = segment_samples(&samples, 1, 2);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 2);
        assert_eq!(groups[1].len(), 2);
    }

    #[test]
    fn to_smooth_ticks_scales() {
        // 1 second at media timescale 90000 → 10_000_000 smooth ticks.
        assert_eq!(SmoothPackager::to_smooth_ticks(90_000, 90_000), 10_000_000);
    }
}
