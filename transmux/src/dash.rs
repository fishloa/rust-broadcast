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
//! - **`SegmentTemplate`** (§5.3.9.4) — number-based addressing with
//!   `initialization` + `media` templates (`$RepresentationID$`, `$Number$`),
//!   `timescale`, `duration`, and `startNumber`.
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

/// MIME type carried on a video `AdaptationSet` (`video/mp4`).
const MIME_VIDEO: &str = "video/mp4";
/// MIME type carried on an audio `AdaptationSet` (`audio/mp4`).
const MIME_AUDIO: &str = "audio/mp4";

/// RFC 6381 `codecs` string for VP8 (WebM codec registration).
const CODECS_VP8: &str = "vp8";
/// RFC 6381 `codecs` string for Vorbis (WebM codec registration).
const CODECS_VORBIS: &str = "vorbis";

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
#[derive(Debug, Clone)]
pub struct DashPackager {
    /// The MPD `profiles` attribute (ISO/IEC 23009-1 §5.3.1.2).
    pub profiles: String,
    /// If `true`, emit `type="dynamic"` (live); otherwise `type="static"` (VOD).
    pub dynamic: bool,
    /// `SegmentTemplate@startNumber` (1-based, ISO/IEC 23009-1 §5.3.9.4.4).
    pub start_number: u64,
    /// `initialization` template (contains `$RepresentationID$`).
    pub init_template: String,
    /// `media` template (contains `$RepresentationID$` and `$Number$`).
    pub media_template: String,
    /// Optional `MPD@availabilityStartTime` (ISO-8601 UTC), live only.
    pub availability_start_time: Option<String>,
    /// Optional `MPD@minimumUpdatePeriod` (xs:duration), live only.
    pub minimum_update_period: Option<String>,
}

impl Default for DashPackager {
    fn default() -> Self {
        Self {
            profiles: PROFILE_ISOFF_LIVE.to_string(),
            dynamic: false,
            start_number: 1,
            init_template: String::from("init-stream$RepresentationID$.m4s"),
            media_template: String::from("chunk-stream$RepresentationID$-$Number$.m4s"),
            availability_start_time: None,
            minimum_update_period: None,
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
        }
    }

    /// Resolve every presentation parameter for one track.
    fn repr_info(track: &Track) -> Result<ReprInfo> {
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

        let mut info = ReprInfo {
            id: track.spec.track_id.to_string(),
            kind,
            codecs,
            bandwidth,
            timescale,
            total_duration,
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
        if self.dynamic {
            if let Some(ast) = &self.availability_start_time {
                mpd_attrs.push(("availabilityStartTime", ast.clone()));
            }
            if let Some(mup) = &self.minimum_update_period {
                mpd_attrs.push(("minimumUpdatePeriod", mup.clone()));
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

        w.close("Period");
        w.close("MPD");
        w.finish()
    }

    fn write_adaptation_set(&self, w: &mut XmlWriter, kind: MediaKind, set: &[&ReprInfo]) {
        let attrs = alloc::vec![
            ("contentType", kind.name().to_string()),
            ("mimeType", kind.mime_type().to_string()),
            ("segmentAlignment", "true".to_string()),
            ("startWithSAP", "1".to_string()),
        ];
        w.open("AdaptationSet", &attrs);

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
        }

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
            reprs.push(Self::repr_info(t)?);
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
