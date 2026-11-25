//! ISO-BMFF / fragmented MP4 (CMAF) codec-identity analysis — GitHub issue
//! #928.
//!
//! Unlike the TS path (`lib.rs`'s `analyze_impl`), `transmux` demuxes the
//! container itself ([`transmux::mp4_demux::Mp4Demux`]): this module only
//! *walks* the
//! resulting [`transmux::Media`]/[`transmux::Track`] IR to report, per track,
//! the codec identity transmux already recovered from the init segment's
//! decoder-config boxes (`avcC`/`hvcC`/`vvcC`/`av1C`/`vpcC`/`esds`/`dOps`/
//! `dfLa`/`dac3`/`dec3`/`dac4`/`mhaC`/`ddts`) — an RFC 6381 codec string plus
//! geometry/channel/sample-rate, sample counts, coded byte totals and an
//! average bitrate. No codec bitstream is ever decoded; only the container's
//! own configuration records (via the crate's own public `rfc6381()`
//! builders, never a re-derived literal).
//!
//! [`transmux::mp4_demux::Mp4Demux`] reads both a fragmented file (`moov` +
//! `moof`/`mdat`, ISO/IEC 14496-12:2026 §8.8) and a classic *progressive*
//! one (a single file with `stbl` sample tables, §8.5-§8.7) — the shape a
//! phone or a `faststart` remux produces. The demo is handed the whole file
//! in browser memory, so it drives the seekable stage over that image.

use broadcast_common::{Parse, Stage, Timestamp};
use serde::Serialize;
use transmux::codec_config_ext::{HevcRecordCodecInfo, VvcRecordCodecInfo};
use transmux::ir::{AbandonReason, CodecConfig, Media, SkippedTrack, Track};
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use transmux::sps::rfc6381_avc1;
use transmux::{AudioSpecificConfig, DemuxEvent};

/// One track's codec identity + coarse throughput stats.
#[derive(Serialize)]
pub struct Mp4Track {
    track_id: u32,
    timescale: u32,
    /// "video" | "audio" | "subtitle" | "data".
    kind: &'static str,
    /// Short codec family label (e.g. "H.265/HEVC", "AAC").
    codec: &'static str,
    /// RFC 6381 `CODECS=` string, when this crate can derive one (every
    /// codec except the opaque [`CodecConfig::Data`] carriage and
    /// [`CodecConfig::Subtitle`], which have no fMP4 sample-entry mapping —
    /// see those variants' doc comments in `transmux::ir::CodecConfig`).
    codec_string: Option<String>,
    width: Option<u16>,
    height: Option<u16>,
    channel_count: Option<u16>,
    sample_rate: Option<u32>,
    sample_count: u64,
    total_bytes: u64,
    duration_seconds: f64,
    /// `total_bytes * 8 / duration_seconds`; `0.0` when duration is `0`
    /// (e.g. a progressive MP4, whose samples this analyzer does not read —
    /// see the module doc comment).
    bitrate_bps: f64,
    encrypted: bool,
    /// `"cenc"` / `"cbcs"` when `encrypted`, else `None`.
    encryption_scheme: Option<&'static str>,
}

/// A track the demuxer found but could not model into a [`Mp4Track`] (an
/// unrecognised sample entry/codec, or a structurally malformed `trak`) —
/// mirrors [`transmux::ir::SkippedTrack`] verbatim so the panel can say
/// exactly what was skipped and why, instead of just under-counting tracks.
#[derive(Serialize)]
pub struct SkippedTrackEntry {
    fourcc: String,
    reason: String,
}

/// Top-level JSON object returned for an MP4/CMAF input.
#[derive(Serialize)]
pub struct Mp4Report {
    /// Always `"mp4"` — lets the UI branch between this report shape and the
    /// TS-oriented [`crate::AnalysisResult`].
    container: &'static str,
    /// Set when `Mp4Demux::unpackage` itself failed (e.g. no `moov` box
    /// found at all) — every other field is then its empty/zero default.
    parse_error: Option<String>,
    movie_timescale: u32,
    tracks: Vec<Mp4Track>,
    skipped_tracks: Vec<SkippedTrackEntry>,
}

/// Coarse track-kind label, mirroring `lib.rs`'s TS-side `PidRole` labels
/// where they overlap ("Video"/"Audio"/"Subtitle") but lower-case, since this
/// is a distinct report shape, not the PID map.
fn track_kind(config: &CodecConfig) -> &'static str {
    match config {
        _ if config.is_video() => "video",
        CodecConfig::Subtitle { .. } => "subtitle",
        CodecConfig::Data { .. } => "data",
        CodecConfig::Aac { .. }
        | CodecConfig::Ac3 { .. }
        | CodecConfig::Eac3 { .. }
        | CodecConfig::Opus { .. }
        | CodecConfig::Flac { .. }
        | CodecConfig::Ac4 { .. }
        | CodecConfig::MpegH { .. }
        | CodecConfig::Dts { .. }
        | CodecConfig::MpegAudio { .. }
        | CodecConfig::Vorbis { .. } => "audio",
        _ => "data",
    }
}

/// Short codec family label for the UI (distinct from `CodecConfig`'s Rust
/// variant name, which isn't meant as a display string).
fn codec_label(config: &CodecConfig) -> &'static str {
    match config {
        CodecConfig::Avc { .. } => "H.264/AVC",
        CodecConfig::Hevc { .. } => "H.265/HEVC",
        CodecConfig::Vvc { .. } => "H.266/VVC",
        CodecConfig::Subtitle { format, .. } => format.name(),
        CodecConfig::Data { .. } => "opaque (unrecognised stream_type)",
        other => other.name(),
    }
}

/// The `objectTypeIndication` carried in an `esds` (`0` if the DecoderConfig
/// is absent) — used to build the RFC 6381 `mp4v.<OTI>` / `mp4a.<OTI>` codec
/// string (ISO/IEC 14496-1 §7.2.6.6), mirroring `transmux::dash`'s own
/// private `oti_of` helper (not reachable from outside the crate) via the
/// same public fields it reads.
fn esds_oti(esds: &isobmff::mp4esds::EsdsBox) -> u8 {
    esds.es_descriptor
        .decoder_config
        .as_ref()
        .map_or(0, |dc| dc.object_type_indication.0)
}

/// Resolve the RFC 6381 codec string for a track, via the crate's own
/// `rfc6381()` builders on each decoder-config box — never a hand-rolled
/// literal. `None` for the two carriage kinds with no fMP4 sample-entry
/// mapping in this crate ([`CodecConfig::Data`], [`CodecConfig::Subtitle`]).
fn codec_string(config: &CodecConfig) -> Option<String> {
    match config {
        CodecConfig::Avc { config, .. } => Some(rfc6381_avc1(
            config.config.profile_indication,
            config.config.profile_compatibility,
            config.config.level_indication,
        )),
        CodecConfig::Hevc { config, .. } => Some(config.config.rfc6381()),
        CodecConfig::Vvc { config, .. } => Some(config.config.rfc6381()),
        CodecConfig::Av1 { config, .. } => Some(config.rfc6381()),
        CodecConfig::Vp9 { config, .. } => Some(config.rfc6381()),
        CodecConfig::Ac3 { config, .. } => Some(config.rfc6381().to_string()),
        CodecConfig::Eac3 { config, .. } => Some(config.rfc6381().to_string()),
        CodecConfig::Opus { config, .. } => Some(config.rfc6381().to_string()),
        CodecConfig::Flac { config, .. } => Some(config.rfc6381().to_string()),
        CodecConfig::Ac4 { config, .. } => Some(config.rfc6381().to_string()),
        CodecConfig::MpegH { config, .. } => Some(config.rfc6381()),
        CodecConfig::Dts { codec_fourcc, .. } => {
            Some(isobmff::DtsSpecificBox::rfc6381(codec_fourcc).to_string())
        }
        CodecConfig::Aac { esds, .. } => esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|dc| dc.decoder_specific_info.as_ref())
            .and_then(|dsi| AudioSpecificConfig::parse(&dsi.data).ok())
            .map(|asc| asc.rfc6381()),
        // RFC 6381 §3.3: MP4 registration uses the sample-entry FourCC plus
        // the ObjectTypeIndication (e.g. `mp4v.61`, `mp4a.6B`).
        CodecConfig::Mpeg2Video { esds, .. } => Some(format!("mp4v.{:02X}", esds_oti(esds))),
        CodecConfig::MpegAudio { esds, .. } => Some(format!("mp4a.{:02X}", esds_oti(esds))),
        // WebM-native codecs (RFC 6386 VP8 / Vorbis I): no ISOBMFF sample
        // entry in this crate, but the bare codec-name token is still the
        // correct WebM-side codecs identifier.
        CodecConfig::Vp8 { .. } => Some("vp8".to_string()),
        CodecConfig::Vorbis { .. } => Some("vorbis".to_string()),
        CodecConfig::Data { .. } | CodecConfig::Subtitle { .. } => None,
        // See the matching wildcard arm in `track_kind` above.
        _ => None,
    }
}

/// Coded geometry, for the video codec families that carry one.
fn dims(config: &CodecConfig) -> (Option<u16>, Option<u16>) {
    match *config {
        CodecConfig::Avc { width, height, .. }
        | CodecConfig::Hevc { width, height, .. }
        | CodecConfig::Vvc { width, height, .. }
        | CodecConfig::Av1 { width, height, .. }
        | CodecConfig::Vp9 { width, height, .. }
        | CodecConfig::Vp8 { width, height, .. }
        | CodecConfig::Mpeg2Video { width, height, .. }
            if width > 0 && height > 0 =>
        {
            (Some(width), Some(height))
        }
        // `0x0` is not a real coded geometry: report it honestly as unknown
        // instead of a misleading "0x0".
        _ => (None, None),
    }
}

/// Channel count + sample rate, for the audio codec families that carry them.
fn audio_params(config: &CodecConfig) -> (Option<u16>, Option<u32>) {
    match config {
        CodecConfig::Aac {
            channel_count,
            sample_rate,
            ..
        }
        | CodecConfig::Ac3 {
            channel_count,
            sample_rate,
            ..
        }
        | CodecConfig::Eac3 {
            channel_count,
            sample_rate,
            ..
        }
        | CodecConfig::Opus {
            channel_count,
            sample_rate,
            ..
        }
        | CodecConfig::Flac {
            channel_count,
            sample_rate,
            ..
        }
        | CodecConfig::Ac4 {
            channel_count,
            sample_rate,
            ..
        }
        | CodecConfig::MpegH {
            channel_count,
            sample_rate,
            ..
        }
        | CodecConfig::Dts {
            channel_count,
            sample_rate,
            ..
        }
        | CodecConfig::MpegAudio {
            channel_count,
            sample_rate,
            ..
        } => (Some(*channel_count), Some(*sample_rate)),
        CodecConfig::Vorbis {
            channels,
            sample_rate,
            ..
        } => (Some(*channels), Some(*sample_rate)),
        _ => (None, None),
    }
}

fn describe_track(track: &Track) -> Mp4Track {
    let config = &track.spec.config;
    let (width, height) = dims(config);
    let (channel_count, sample_rate) = audio_params(config);

    let sample_count = track.samples.len() as u64;
    let total_bytes: u64 = track.samples.iter().map(|s| s.data.len() as u64).sum();
    let total_ticks: u64 = track
        .samples
        .iter()
        .map(|s| u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)))
        .sum();
    let timescale = track.spec.timescale.get();
    let duration_seconds = total_ticks as f64 / f64::from(timescale);
    let bitrate_bps = if duration_seconds > 0.0 {
        (total_bytes as f64 * 8.0) / duration_seconds
    } else {
        0.0
    };

    let (encrypted, encryption_scheme) = match &track.spec.protection {
        Some(enc) => (true, Some(enc.scheme.name())),
        None => (false, None),
    };

    Mp4Track {
        track_id: track.track_id().value(),
        timescale: track.spec.timescale.get(),
        kind: track_kind(config),
        codec: codec_label(config),
        codec_string: codec_string(config),
        width,
        height,
        channel_count,
        sample_rate,
        sample_count,
        total_bytes,
        duration_seconds,
        bitrate_bps,
        encrypted,
        encryption_scheme,
    }
}

fn build_report(media: Media) -> Mp4Report {
    let tracks = media.tracks.iter().map(describe_track).collect();
    let skipped_tracks = media
        .skipped
        .into_iter()
        .map(|s| SkippedTrackEntry {
            fourcc: s.fourcc,
            reason: s.reason,
        })
        .collect();
    Mp4Report {
        container: "mp4",
        parse_error: None,
        movie_timescale: media.movie_timescale.get(),
        tracks,
        skipped_tracks,
    }
}

/// Demux an in-memory MP4 image through the streaming stage.
///
/// The demo is handed the whole file in browser memory, so it drives the
/// seekable stage over that image; no second copy of the file is made.
fn demux_mp4(bytes: &[u8]) -> Result<Media, transmux::Error> {
    let total_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let config = Mp4DemuxConfig::default().with_total_len(Some(total_len));
    let mut demux = Mp4Demux::seekable(config);
    loop {
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(demand.want_bytes).min(bytes.len());
        let slice = bytes.get(start..end).unwrap_or(&[]);
        demux.feed(
            Mp4Input::At {
                offset,
                bytes: slice.to_vec().into(),
            },
            Timestamp::ZERO,
        )?;
    }
    demux.finish()?;
    if demux.is_fragmented().is_none() {
        return Err(transmux::Error::UnexpectedBox { expected: "moov" });
    }
    let movie_timescale = demux
        .movie_timescale()
        .unwrap_or(transmux::VIDEO_CLOCK_TIMESCALE);
    let mut tracks: Vec<Track> = Vec::new();
    let mut index: std::collections::BTreeMap<transmux::TrackId, usize> =
        std::collections::BTreeMap::new();
    let mut skipped = Vec::new();
    let mut degraded = Vec::new();
    while let Some(event) = demux.poll() {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                index.insert(spec.track_id, tracks.len());
                tracks.push(Track::new(spec, Vec::new()));
            }
            DemuxEvent::TrackUpdated(spec) => {
                if let Some(&at) = index.get(&spec.track_id) {
                    tracks[at].spec = spec;
                }
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(&at) = index.get(&track_id) {
                    tracks[at].samples.push(sample);
                }
            }
            DemuxEvent::TrackAbandoned {
                track_id, reason, ..
            } => skipped.push(skipped_from_reason(reason, track_id)),
            DemuxEvent::InputDegraded { kind, .. } => degraded.push(kind),
            _ => {}
        }
    }
    let mut media = Media::new(tracks, movie_timescale);
    media.skipped = skipped;
    media.degraded = degraded;
    Ok(media)
}

fn skipped_from_reason(reason: AbandonReason, track_id: Option<transmux::TrackId>) -> SkippedTrack {
    let (fourcc, text) = match reason {
        AbandonReason::UnsupportedSampleEntry { fourcc, detail } => {
            let fourcc = String::from_utf8_lossy(&fourcc).into_owned();
            let text = match detail {
                Some(detail) => format!("sample entry '{fourcc}' rejected: {detail}"),
                None => format!(
                    "sample entry '{fourcc}' has no CodecConfig reconstruction in this crate"
                ),
            };
            (fourcc, text)
        }
        AbandonReason::UnsupportedCodec => (
            String::from("unknown"),
            "sample entry 'unknown' has no CodecConfig reconstruction in this crate".into(),
        ),
        AbandonReason::MalformedTrack { expected } => (
            String::from("unknown"),
            format!("unexpected box: expected {expected}"),
        ),
        other => (String::from("unknown"), other.name().into()),
    };
    let mut skipped = SkippedTrack::new(fourcc, text);
    skipped.track_id = track_id;
    skipped
}

/// Run the MP4/CMAF codec-identity pass over a raw ISO-BMFF byte buffer.
///
/// Never panics: a demux failure (e.g. no `moov` box) is reported in
/// `parse_error`, not raised. A CENC/CBCS-protected `encv`/`enca` entry is
/// demuxed as its real codec with `TrackSpec::protection` set, so it is
/// reported encrypted with its scheme rather than skipped.
pub fn analyze_mp4_impl(bytes: &[u8]) -> Mp4Report {
    let media = match demux_mp4(bytes) {
        Ok(media) => media,
        Err(e) => {
            return Mp4Report {
                container: "mp4",
                parse_error: Some(e.to_string()),
                movie_timescale: 0,
                tracks: Vec::new(),
                skipped_tracks: Vec::new(),
            };
        }
    };
    build_report(media)
}


