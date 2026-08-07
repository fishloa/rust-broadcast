//! Matroska (MKV) muxer round-trip integration tests (issue #915).
//!
//! Each fixture is demuxed with [`WebmDemux`] (real ffmpeg captures — see
//! `fixtures/mkv/GENERATE.md`), packaged back to bytes with [`MkvMux`], then
//! re-demuxed with the same [`WebmDemux`]. The two [`Media`] values (original
//! demux vs. demux-of-the-remux) are compared field-by-field: track count,
//! codec identity/config, sample count, PTS/DTS, keyframe flags, and the
//! coded sample bytes themselves. A muxer that dropped a sample, mis-scaled a
//! timestamp, or corrupted `CodecPrivate` fails one of these comparisons.

use broadcast_common::Package;
use transmux::pipeline::CodecConfig;
use transmux::webm_demux::WebmDemux;
use transmux::{Media, MkvMux};

/// H.264 (`fixtures/ts/h264_aac.ts`, `-c copy`'d to Matroska) + AAC.
const H264_AAC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mkv/h264_aac.mkv");
/// HEVC (`fixtures/ts/hevc/main.ts`) video muxed with the same real AAC audio
/// as [`H264_AAC`] (`fixtures/ts/h264_aac.ts`) — two genuine captures, one file.
const HEVC_AAC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mkv/hevc_aac.mkv");
/// VP9 + Opus (`fixtures/webm/vp9_opus.webm`, `-c copy`'d to Matroska).
const VP9_OPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mkv/vp9_opus.mkv");

fn demux(path: &str) -> Media {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    WebmDemux::new()
        .demux(&bytes)
        .unwrap_or_else(|e| panic!("demux {path}: {e}"))
}

/// Demux `path`, mux the result back with [`MkvMux`], then demux that output
/// again. Returns (original demux, demux-of-the-remux).
fn round_trip(path: &str) -> (Media, Media) {
    let original = demux(path);
    let muxed = MkvMux::new()
        .package(&original)
        .unwrap_or_else(|e| panic!("mux {path}: {e}"));
    let remuxed = WebmDemux::new()
        .demux(&muxed)
        .unwrap_or_else(|e| panic!("re-demux muxed {path}: {e}"));
    (original, remuxed)
}

/// A short discriminant name for a [`CodecConfig`], for the codec-kind
/// assertion (the detailed field comparison happens separately per variant).
fn codec_kind(c: &CodecConfig) -> &'static str {
    match c {
        CodecConfig::Avc { .. } => "Avc",
        CodecConfig::Hevc { .. } => "Hevc",
        CodecConfig::Vp9 { .. } => "Vp9",
        CodecConfig::Vp8 { .. } => "Vp8",
        CodecConfig::Av1 { .. } => "Av1",
        CodecConfig::Aac { .. } => "Aac",
        CodecConfig::Opus { .. } => "Opus",
        CodecConfig::Vorbis { .. } => "Vorbis",
        _ => "other",
    }
}

/// Assert the two `CodecConfig`s carry byte-identical `CodecPrivate`-equivalent
/// config plus matching dimensions/channels/sample-rate.
fn assert_codec_config_equal(a: &CodecConfig, b: &CodecConfig) {
    match (a, b) {
        (
            CodecConfig::Avc {
                config: ca,
                width: wa,
                height: ha,
            },
            CodecConfig::Avc {
                config: cb,
                width: wb,
                height: hb,
            },
        ) => {
            use broadcast_common::Serialize;
            assert_eq!(ca.config.to_bytes(), cb.config.to_bytes(), "avcC bytes");
            assert_eq!((wa, ha), (wb, hb), "AVC dimensions");
        }
        (
            CodecConfig::Hevc {
                config: ca,
                width: wa,
                height: ha,
            },
            CodecConfig::Hevc {
                config: cb,
                width: wb,
                height: hb,
            },
        ) => {
            use broadcast_common::Serialize;
            assert_eq!(ca.config.to_bytes(), cb.config.to_bytes(), "hvcC bytes");
            assert_eq!((wa, ha), (wb, hb), "HEVC dimensions");
        }
        (
            CodecConfig::Vp9 {
                width: wa,
                height: ha,
                ..
            },
            CodecConfig::Vp9 {
                width: wb,
                height: hb,
                ..
            },
        ) => {
            assert_eq!((wa, ha), (wb, hb), "VP9 dimensions");
        }
        (
            CodecConfig::Aac {
                esds: ea,
                channel_count: cca,
                sample_rate: sra,
                ..
            },
            CodecConfig::Aac {
                esds: eb,
                channel_count: ccb,
                sample_rate: srb,
                ..
            },
        ) => {
            let asc_a = &ea
                .es_descriptor
                .decoder_config
                .as_ref()
                .unwrap()
                .decoder_specific_info
                .as_ref()
                .unwrap()
                .data;
            let asc_b = &eb
                .es_descriptor
                .decoder_config
                .as_ref()
                .unwrap()
                .decoder_specific_info
                .as_ref()
                .unwrap()
                .data;
            assert_eq!(asc_a, asc_b, "AAC AudioSpecificConfig bytes");
            assert_eq!((cca, sra), (ccb, srb), "AAC channels/rate");
        }
        (
            CodecConfig::Opus {
                channel_count: cca,
                sample_rate: sra,
                ..
            },
            CodecConfig::Opus {
                channel_count: ccb,
                sample_rate: srb,
                ..
            },
        ) => {
            assert_eq!((cca, sra), (ccb, srb), "Opus channels/rate");
        }
        (a, b) => panic!("codec kind mismatch: {a:?} vs {b:?}"),
    }
}

/// Assert two [`Media`] values are equivalent for round-trip purposes: same
/// track count/order, same codec identity/config per track, same sample
/// count/timing/keyframe-flags/coded-bytes per track.
fn assert_media_round_trips(original: &Media, remuxed: &Media) {
    assert_eq!(
        original.tracks.len(),
        remuxed.tracks.len(),
        "track count differs after mux/re-demux"
    );
    for (i, (ta, tb)) in original.tracks.iter().zip(&remuxed.tracks).enumerate() {
        assert_eq!(
            codec_kind(&ta.spec.config),
            codec_kind(&tb.spec.config),
            "track {i} codec kind"
        );
        assert_codec_config_equal(&ta.spec.config, &tb.spec.config);
        assert_eq!(ta.samples.len(), tb.samples.len(), "track {i} sample count");
        for (j, (sa, sb)) in ta.samples.iter().zip(&tb.samples).enumerate() {
            assert_eq!(sa.pts, sb.pts, "track {i} sample {j} pts");
            assert_eq!(sa.dts, sb.dts, "track {i} sample {j} dts");
            assert_eq!(
                sa.flags.is_sync, sb.flags.is_sync,
                "track {i} sample {j} keyframe flag"
            );
            assert_eq!(sa.data, sb.data, "track {i} sample {j} coded bytes");
        }
    }
}

#[test]
fn mkv_round_trip_h264_aac() {
    let (original, remuxed) = round_trip(H264_AAC);
    assert_eq!(original.tracks.len(), 2, "expected video + audio tracks");
    assert!(matches!(
        original.tracks[0].spec.config,
        CodecConfig::Avc { .. }
    ));
    assert!(matches!(
        original.tracks[1].spec.config,
        CodecConfig::Aac { .. }
    ));
    assert!(!original.tracks[0].samples.is_empty());
    assert!(!original.tracks[1].samples.is_empty());
    assert_media_round_trips(&original, &remuxed);
}

#[test]
fn mkv_round_trip_hevc_aac() {
    let (original, remuxed) = round_trip(HEVC_AAC);
    assert_eq!(original.tracks.len(), 2, "expected video + audio tracks");
    assert!(matches!(
        original.tracks[0].spec.config,
        CodecConfig::Hevc { .. }
    ));
    assert!(matches!(
        original.tracks[1].spec.config,
        CodecConfig::Aac { .. }
    ));
    assert!(!original.tracks[0].samples.is_empty());
    assert!(!original.tracks[1].samples.is_empty());
    assert_media_round_trips(&original, &remuxed);
}

#[test]
fn mkv_round_trip_vp9_opus() {
    let (original, remuxed) = round_trip(VP9_OPUS);
    assert_eq!(original.tracks.len(), 2, "expected video + audio tracks");
    assert!(matches!(
        original.tracks[0].spec.config,
        CodecConfig::Vp9 { .. }
    ));
    assert!(matches!(
        original.tracks[1].spec.config,
        CodecConfig::Opus { .. }
    ));
    assert!(!original.tracks[0].samples.is_empty());
    assert!(!original.tracks[1].samples.is_empty());
    assert_media_round_trips(&original, &remuxed);
}

/// A muxed file must itself be well-formed enough to survive a *second*
/// mux/re-demux cycle byte-identically (idempotence beyond one round trip).
#[test]
fn mkv_double_round_trip_is_stable() {
    let (_, remuxed_once) = round_trip(H264_AAC);
    let muxed_twice = MkvMux::new().package(&remuxed_once).expect("second mux");
    let remuxed_twice = WebmDemux::new()
        .demux(&muxed_twice)
        .expect("second re-demux");
    assert_media_round_trips(&remuxed_once, &remuxed_twice);
}
