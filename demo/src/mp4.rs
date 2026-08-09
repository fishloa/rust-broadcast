//! ISO-BMFF / fragmented MP4 (CMAF) codec-identity analysis — GitHub issue
//! #928.
//!
//! Unlike the TS path (`lib.rs`'s `analyze_impl`), `transmux` demuxes the
//! container itself ([`transmux::Fmp4Demux`]): this module only *walks* the
//! resulting [`transmux::Media`]/[`transmux::Track`] IR to report, per track,
//! the codec identity transmux already recovered from the init segment's
//! decoder-config boxes (`avcC`/`hvcC`/`vvcC`/`av1C`/`vpcC`/`esds`/`dOps`/
//! `dfLa`/`dac3`/`dec3`/`dac4`/`mhaC`/`ddts`) — an RFC 6381 codec string plus
//! geometry/channel/sample-rate, sample counts, coded byte totals and an
//! average bitrate. No codec bitstream is ever decoded; only the container's
//! own configuration records (via the crate's own public `rfc6381()`
//! builders, never a re-derived literal).
//!
//! [`transmux::Fmp4Demux`] only reads `moov` + `moof`/`mdat` fragments (ISO/IEC
//! 14496-12 §8.8); a classic *progressive* (single-file, non-fragmented,
//! `stbl`-sample-table) MP4 — the common shape a phone or a `faststart`
//! remux produces, with no `moof` at all — needs the sibling
//! [`transmux::ProgressiveDemux`] (ISO/IEC 14496-12 §8.5-§8.7) instead. This
//! module picks between the two per input by checking for a top-level `moof`
//! box ([`has_top_level_moof`]) rather than guessing from a demux failure,
//! since an all-`moov`-no-`moof` input is not an *error* for [`Fmp4Demux`] —
//! it successfully returns a `Media` with correct codec identity but zero
//! samples per track (no fragments to read), which would silently look like
//! a working but sample-less file instead of "wrong demuxer for this shape".

use broadcast_common::{Parse, Unpackage};
use serde::Serialize;

use transmux::ir::{CodecConfig, Media, Track};
use transmux::sps::rfc6381_avc1;
use transmux::{AudioSpecificConfig, Fmp4Demux, ProgressiveDemux, box_iter};

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
    /// Set when `Fmp4Demux::unpackage` itself failed (e.g. no `moov` box
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
        CodecConfig::Avc { .. }
        | CodecConfig::Hevc { .. }
        | CodecConfig::Vvc { .. }
        | CodecConfig::Av1 { .. }
        | CodecConfig::Vp9 { .. }
        | CodecConfig::Vp8 { .. }
        | CodecConfig::Mpeg2Video { .. } => "video",
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
        CodecConfig::Subtitle { .. } => "subtitle",
        CodecConfig::Data { .. } => "data",
        // `CodecConfig` is `#[non_exhaustive]`: a future variant this crate
        // hasn't been updated for yet falls back to "data" rather than
        // failing to compile against a newer transmux.
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
        CodecConfig::Av1 { .. } => "AV1",
        CodecConfig::Vp9 { .. } => "VP9",
        CodecConfig::Vp8 { .. } => "VP8",
        CodecConfig::Mpeg2Video { .. } => "MPEG-2 Video",
        CodecConfig::Aac { .. } => "AAC",
        CodecConfig::Ac3 { .. } => "AC-3",
        CodecConfig::Eac3 { .. } => "E-AC-3",
        CodecConfig::Opus { .. } => "Opus",
        CodecConfig::Flac { .. } => "FLAC",
        CodecConfig::Ac4 { .. } => "AC-4",
        CodecConfig::MpegH { .. } => "MPEG-H 3D Audio",
        CodecConfig::Dts { .. } => "DTS",
        CodecConfig::MpegAudio { .. } => "MPEG-1/2 Audio",
        CodecConfig::Vorbis { .. } => "Vorbis",
        CodecConfig::Subtitle { format } => format.name(),
        CodecConfig::Data { .. } => "opaque (unrecognised stream_type)",
        // See the matching wildcard arm in `track_kind` above.
        _ => "unrecognised (unsupported by this analyzer build)",
    }
}

/// The `objectTypeIndication` carried in an `esds` (`0` if the DecoderConfig
/// is absent) — used to build the RFC 6381 `mp4v.<OTI>` / `mp4a.<OTI>` codec
/// string (ISO/IEC 14496-1 §7.2.6.6), mirroring `transmux::dash`'s own
/// private `oti_of` helper (not reachable from outside the crate) via the
/// same public fields it reads.
fn esds_oti(esds: &transmux::mp4esds::EsdsBox) -> u8 {
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
            Some(transmux::DtsSpecificBox::rfc6381(codec_fourcc).to_string())
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
        // `0x0` is not a real coded geometry — `transmux::CencDecryptor`'s
        // protected-AVC recovery path (see `analyze_mp4_impl`) only
        // reconstructs the `avcC` config record, not the sample entry's
        // `width`/`height` fields, and reports the literal `0` for both
        // rather than fabricating a value; report that honestly as unknown
        // too, instead of a misleading "0x0".
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

/// `describe_track`'s `protected_scheme` argument: the CENC/CBCS scheme name
/// for this track's ID, when [`analyze_mp4_impl`]'s separate
/// [`transmux::CencDecryptor`] pass recovered one (see that function's doc
/// comment — [`Track::encryption`] itself is never populated by any of this
/// crate's *demux* paths, only by [`transmux::CencEncryptor`] on the encode
/// side, so this side channel is this analyzer's own bridge, not a
/// transmux-native field).
fn describe_track(track: &Track, protected_scheme: Option<&'static str>) -> Mp4Track {
    let config = &track.spec.config;
    let (width, height) = dims(config);
    let (channel_count, sample_rate) = audio_params(config);

    let sample_count = track.samples.len() as u64;
    let total_bytes: u64 = track.samples.iter().map(|s| s.data.len() as u64).sum();
    let total_ticks: u64 = track
        .samples
        .iter()
        .map(|s| u64::from(s.duration.unwrap_or(0)))
        .sum();
    let timescale = track.spec.timescale.max(1);
    let duration_seconds = total_ticks as f64 / f64::from(timescale);
    let bitrate_bps = if duration_seconds > 0.0 {
        (total_bytes as f64 * 8.0) / duration_seconds
    } else {
        0.0
    };

    let (encrypted, encryption_scheme) = match (&track.encryption, protected_scheme) {
        (Some(enc), _) => (true, Some(enc.scheme.name())),
        (None, Some(scheme)) => (true, Some(scheme)),
        (None, None) => (false, None),
    };

    Mp4Track {
        track_id: track.track_id(),
        timescale: track.spec.timescale,
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

fn build_report(
    media: Media,
    protected_track_id: Option<u32>,
    protected_scheme: Option<&'static str>,
) -> Mp4Report {
    let tracks = media
        .tracks
        .iter()
        .map(|t| {
            let scheme = if Some(t.track_id()) == protected_track_id {
                protected_scheme
            } else {
                None
            };
            describe_track(t, scheme)
        })
        .collect();
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
        movie_timescale: media.movie_timescale,
        tracks,
        skipped_tracks,
    }
}

/// ISO/IEC 14496-12 §8.8.4: whether any top-level box in `bytes` is a
/// `moof` (movie fragment) — the presence test that decides fragmented
/// ([`Fmp4Demux`]) vs. progressive ([`ProgressiveDemux`]) demux below.
/// Malformed/truncated boxes encountered while walking simply end the scan
/// (`box_iter` stops yielding on the first parse error) rather than
/// panicking; a file that turns out unreadable either way surfaces as a
/// `parse_error` from whichever demuxer is then tried.
fn has_top_level_moof(bytes: &[u8]) -> bool {
    box_iter(bytes)
        .filter_map(|r| r.ok())
        .any(|(box_ref, _consumed)| box_ref.header.box_type.is(b"moof"))
}

/// Run the MP4/CMAF codec-identity pass over a raw ISO-BMFF byte buffer.
///
/// Never panics: a demux failure (e.g. no `moov` box) is reported in
/// `parse_error`, not raised.
///
/// A second, independent pass handles CENC/CBCS-protected tracks: neither
/// [`Fmp4Demux`] nor [`ProgressiveDemux`] reconstructs an `encv`/`enca`
/// sample entry (there is no `sinf`/`tenc` unwrap in either — that logic
/// lives only in [`transmux::CencDecryptor`], the crate's dedicated *decrypt*
/// path) — a protected track from the primary pass above lands in
/// `skipped_tracks` naming the raw `encv`/`enca` fourcc. [`CencDecryptor`]
/// is tried next, purely to *identify* such a track (`from_fmp4` harvests
/// `sinf`/`tenc`/`senc` metadata; `demux` returns the still-encrypted
/// samples) — no decryption key is ever supplied or needed. It currently
/// reconstructs protected **AVC video only** (`demux_protected`'s own
/// documented scope, and its single-track accessors), so this is only
/// applied when it recovers exactly one track and the primary pass had
/// exactly one matching `encv`/`enca` skip — otherwise a mixed/ambiguous
/// file is left as-is rather than guessing which skipped track a
/// single-track accessor's scheme belongs to. A protected HEVC or audio
/// track, or a file with more than one protected track, therefore still
/// surfaces honestly via `skipped_tracks` — this analyzer does not invent an
/// identity transmux itself cannot recover.
pub fn analyze_mp4_impl(bytes: &[u8]) -> Mp4Report {
    let result = if has_top_level_moof(bytes) {
        Fmp4Demux::new().unpackage(bytes)
    } else {
        // The inherent `Unpackage::unpackage` path never consults
        // `max_bytes` (it borrows the caller's slice directly rather than
        // accumulating into an owned buffer) — see `ProgressiveDemux::new`'s
        // doc comment — so any non-zero cap is equally correct here; using
        // the input's own length keeps the value self-explanatory rather
        // than an arbitrary constant.
        match ProgressiveDemux::new(bytes.len().max(1)) {
            Ok(mut demux) => demux.unpackage(bytes),
            Err(e) => Err(e),
        }
    };

    let mut media = match result {
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

    let encv_or_enca_skips = media
        .skipped
        .iter()
        .filter(|s| s.fourcc == "encv" || s.fourcc == "enca")
        .count();
    let mut protected_scheme: Option<&'static str> = None;
    if encv_or_enca_skips == 1 {
        if let Ok(decryptor) = transmux::CencDecryptor::from_fmp4(bytes) {
            if let Ok(mut protected) = decryptor.demux() {
                if protected.tracks.len() == 1 {
                    media
                        .skipped
                        .retain(|s| s.fourcc != "encv" && s.fourcc != "enca");
                    protected_scheme = decryptor.scheme().map(|s| s.name());
                    media.tracks.push(protected.tracks.remove(0));
                }
            }
        }
    }

    let protected_track_id = protected_scheme.map(|_| {
        media
            .tracks
            .last()
            .expect("just pushed the recovered protected track above")
            .track_id()
    });

    build_report(media, protected_track_id, protected_scheme)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/{}"),
            name
        );
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"))
    }

    fn transmux_fixture(name: &str) -> Vec<u8> {
        let path = format!(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/transmux/{}"),
            name
        );
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"))
    }

    /// Real-fixture gate: `h264_high.mp4` is a *progressive* (faststart,
    /// `moov` after a single `mdat`, no `moof`) capture — this exercises the
    /// [`ProgressiveDemux`] branch of [`analyze_mp4_impl`]'s dispatch and
    /// must resolve a video track carrying a well-formed `avc1.` RFC 6381
    /// string and at least one decoded sample (progressive files carry full
    /// `stbl` sample tables, unlike a `moof`-less input with no fragments at
    /// all, which would legitimately have zero samples).
    #[test]
    fn h264_high_progressive_resolves_codec_identity() {
        let bytes = fixture("h264_high.mp4");
        let report = analyze_mp4_impl(&bytes);
        assert!(report.parse_error.is_none(), "{:?}", report.parse_error);
        let video = report
            .tracks
            .iter()
            .find(|t| t.kind == "video")
            .expect("expected a video track");
        assert_eq!(video.codec, "H.264/AVC");
        let cs = video
            .codec_string
            .as_deref()
            .expect("expected a codec string");
        assert!(cs.starts_with("avc1."), "got {cs}");
        assert!(video.sample_count > 0, "expected at least one sample");
        assert!(video.width.unwrap_or(0) > 0);
        assert!(video.height.unwrap_or(0) > 0);
    }

    /// Real-fixture gate for the OTHER dispatch branch: `h264_aac_frag.mp4`
    /// (under `fixtures/transmux/`) is a genuinely fragmented CMAF capture
    /// (`moov` + three `moof`/`mdat` pairs) — this exercises
    /// [`analyze_mp4_impl`]'s [`Fmp4Demux`] branch, on both a video and an
    /// audio track, with samples spanning multiple fragments.
    #[test]
    fn h264_aac_fragmented_resolves_both_tracks_across_fragments() {
        let bytes = transmux_fixture("h264_aac_frag.mp4");
        let report = analyze_mp4_impl(&bytes);
        assert!(report.parse_error.is_none(), "{:?}", report.parse_error);

        let video = report
            .tracks
            .iter()
            .find(|t| t.kind == "video")
            .expect("expected a video track");
        assert_eq!(video.codec, "H.264/AVC");
        assert!(
            video.sample_count > 0,
            "expected samples from moof/mdat fragments"
        );
        assert!(video.width.unwrap_or(0) > 0);

        let audio = report
            .tracks
            .iter()
            .find(|t| t.kind == "audio")
            .expect("expected an audio track");
        assert_eq!(audio.codec, "AAC");
        assert!(audio.sample_count > 0);
        let cs = audio
            .codec_string
            .as_deref()
            .expect("expected an AAC codec string");
        assert!(cs.starts_with("mp4a."), "got {cs}");
    }

    /// HEVC real fixture: `hvc1`/`hev1` + `hvcC` → an `hvc1.` (or `hev1.`)
    /// RFC 6381 string.
    #[test]
    fn hevc_fragment_resolves_codec_identity() {
        let bytes = fixture("hevc_main.mp4");
        let report = analyze_mp4_impl(&bytes);
        assert!(report.parse_error.is_none(), "{:?}", report.parse_error);
        let video = report
            .tracks
            .iter()
            .find(|t| t.kind == "video")
            .expect("expected a video track");
        assert_eq!(video.codec, "H.265/HEVC");
        assert!(video.codec_string.is_some());
    }

    /// AV1 real fixture, exercising the `esds`-free `av1C` path.
    #[test]
    fn av1_fragment_resolves_codec_identity() {
        let bytes = fixture("av1.mp4");
        let report = analyze_mp4_impl(&bytes);
        assert!(report.parse_error.is_none(), "{:?}", report.parse_error);
        let video = report
            .tracks
            .iter()
            .find(|t| t.kind == "video")
            .expect("expected a video track");
        assert_eq!(video.codec, "AV1");
        assert!(
            video
                .codec_string
                .as_deref()
                .unwrap_or("")
                .starts_with("av01.")
        );
    }

    /// CENC-encrypted real fixture: the track must be reported `encrypted`
    /// with a named scheme, not silently dropped or misreported cleartext.
    #[test]
    fn cenc_fixture_reports_encryption() {
        let bytes = fixture("cenc.mp4");
        let report = analyze_mp4_impl(&bytes);
        assert!(report.parse_error.is_none(), "{:?}", report.parse_error);
        let encrypted_track = report
            .tracks
            .iter()
            .find(|t| t.encrypted)
            .expect("expected at least one encrypted track");
        assert!(
            matches!(
                encrypted_track.encryption_scheme,
                Some("cenc") | Some("cbcs")
            ),
            "got {:?}",
            encrypted_track.encryption_scheme
        );
    }

    /// Opus/FLAC audio fixtures resolve channel/sample-rate + a codec string.
    #[test]
    fn opus_fixture_resolves_audio_identity() {
        let bytes = fixture("opus.mp4");
        let report = analyze_mp4_impl(&bytes);
        assert!(report.parse_error.is_none(), "{:?}", report.parse_error);
        let audio = report
            .tracks
            .iter()
            .find(|t| t.kind == "audio")
            .expect("expected an audio track");
        assert_eq!(audio.codec, "Opus");
        // RFC 6381 registration for Opus is the literal (capitalized)
        // "Opus" — see `OpusSpecificBox::rfc6381`'s own doc comment.
        assert_eq!(audio.codec_string.as_deref(), Some("Opus"));
        assert!(audio.channel_count.unwrap_or(0) > 0);
        assert!(audio.sample_rate.unwrap_or(0) > 0);
    }

    /// Garbage input must never panic and must report the failure honestly.
    #[test]
    fn garbage_input_never_panics() {
        let garbage = vec![0u8; 4096];
        let report = analyze_mp4_impl(&garbage);
        assert!(report.parse_error.is_some());
        assert!(report.tracks.is_empty());
    }
}
