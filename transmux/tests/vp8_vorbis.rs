//! VP8 + Vorbis (WebM) demux integration tests — oracle-driven.
//!
//! Completes WebM codec coverage alongside the VP9/Opus suite
//! (`tests/webm_demux.rs`). Two real ffmpeg-produced fixtures, each with a
//! per-frame ffprobe oracle:
//!
//! - `fixtures/webm/vp8_opus.webm` — VP8 video (25 frames) + Opus audio (51).
//! - `fixtures/webm/vorbis.webm` — Vorbis audio (45 frames).
//!
//! Oracle CSV columns (`fixtures/webm/*.packets.csv`):
//!
//! ```text
//! codec_type,stream_index,pts,dts,duration,size,keyframe(K=1)
//! ```
//!
//! The `size` column is the per-frame coded byte length — the strong bite:
//! wrong VINT / block / lacing parsing yields wrong sizes. Dimensions
//! (320×240) are parsed from the VP8 key-frame header (RFC 6386 §9.1) and the
//! Vorbis channels/sample_rate from the `CodecPrivate` identification header
//! (Vorbis I §4.2.2) — never hardcoded into the parse.

use transmux::pipeline::CodecConfig;
use transmux::webm_demux::WebmDemux;
use transmux::Media;

/// VP8+Opus fixture (video track 0, audio track 1).
const VP8_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/webm/vp8_opus.webm"
);
/// VP8+Opus ffprobe oracle.
const VP8_ORACLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/webm/vp8_opus.packets.csv"
);
/// Vorbis-only fixture (audio track 0).
const VORBIS_FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/webm/vorbis.webm");
/// Vorbis ffprobe oracle.
const VORBIS_ORACLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/webm/vorbis.packets.csv"
);

/// One ffprobe oracle row.
#[derive(Debug)]
struct OracleRow {
    codec_type: String,
    size: usize,
    keyframe: bool,
}

fn load_oracle(path: &str) -> Vec<OracleRow> {
    let text = std::fs::read_to_string(path).expect("read oracle csv");
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        rows.push(OracleRow {
            codec_type: f[0].to_string(),
            size: f[5].trim().parse().unwrap(),
            keyframe: f[6].trim() == "1",
        });
    }
    rows
}

fn oracle_for<'a>(rows: &'a [OracleRow], codec_type: &str) -> Vec<&'a OracleRow> {
    rows.iter().filter(|r| r.codec_type == codec_type).collect()
}

fn demux(path: &str) -> Media {
    let bytes = std::fs::read(path).expect("read webm fixture");
    let mut d = WebmDemux::new();
    d.demux(&bytes).expect("demux webm")
}

/// Test 1 — VP8 enumeration + dimensions decoded from the key-frame header.
/// Track 0 is `Vp8` with 320×240 parsed from the keyframe header (RFC 6386
/// §9.1), track 1 is `Opus`.
#[test]
fn vp8_enumeration_and_dimensions() {
    let m = demux(VP8_FIXTURE);
    assert_eq!(m.tracks.len(), 2, "expected VP8 video + Opus audio");

    let CodecConfig::Vp8 { width, height } = &m.tracks[0].spec.config else {
        panic!("track 0 must be VP8, got {:?}", m.tracks[0].spec.config);
    };
    // Decoded from the VP8 key-frame header, not hardcoded into the parser.
    assert_eq!(*width, 320, "VP8 width from key-frame header");
    assert_eq!(*height, 240, "VP8 height from key-frame header");

    assert!(
        matches!(m.tracks[1].spec.config, CodecConfig::Opus { .. }),
        "track 1 must be Opus, got {:?}",
        m.tracks[1].spec.config
    );
}

/// Test 2 — VP8 frame oracle: 25 video frames; each sample byte length equals
/// the CSV `size`, in order; keyframe flag matches the CSV `keyframe` column
/// (exactly one keyframe, the first).
#[test]
fn vp8_frame_oracle() {
    let m = demux(VP8_FIXTURE);
    let oracle = load_oracle(VP8_ORACLE);
    let vid_oracle = oracle_for(&oracle, "video");

    assert_eq!(vid_oracle.len(), 25, "oracle sanity: 25 VP8 frames");
    let vid = &m.tracks[0];
    assert_eq!(vid.samples.len(), 25, "VP8 sample count");

    for (i, (s, o)) in vid.samples.iter().zip(vid_oracle.iter()).enumerate() {
        assert_eq!(
            s.data.len(),
            o.size,
            "VP8 sample {i} byte length must equal oracle size"
        );
        assert_eq!(
            s.is_sync, o.keyframe,
            "VP8 sample {i} keyframe flag must equal oracle"
        );
    }
    // The first frame is the only keyframe (a bite on the SimpleBlock flag ↔
    // key_frame mapping).
    assert!(vid.samples[0].is_sync, "first VP8 frame is a keyframe");
    assert_eq!(
        vid.samples.iter().filter(|s| s.is_sync).count(),
        1,
        "exactly one VP8 keyframe"
    );
}

/// Test 3 — Vorbis enumeration + config decoded from the `CodecPrivate`
/// identification header. `channels`/`sample_rate` come from the header
/// (Vorbis I §4.2.2); `codec_private` is the verbatim Xiph-laced 3-header blob
/// whose first byte is the lacing count (`numPackets-1 == 2`).
#[test]
fn vorbis_enumeration_and_config() {
    let m = demux(VORBIS_FIXTURE);
    assert_eq!(m.tracks.len(), 1, "expected one Vorbis audio track");

    let CodecConfig::Vorbis {
        codec_private,
        channels,
        sample_rate,
    } = &m.tracks[0].spec.config
    else {
        panic!("track 0 must be Vorbis, got {:?}", m.tracks[0].spec.config);
    };

    // Decoded from the identification header (real fixture values).
    assert_eq!(*channels, 2, "Vorbis channels from identification header");
    assert_eq!(
        *sample_rate, 44_100,
        "Vorbis sample rate from identification header"
    );

    // CodecPrivate carried verbatim: non-empty and starts with the Xiph lacing
    // count byte (numPackets-1 == 2 → three headers).
    assert!(!codec_private.is_empty(), "CodecPrivate must be non-empty");
    assert_eq!(
        codec_private[0], 2,
        "CodecPrivate begins with the Xiph lacing count (numPackets-1 == 2)"
    );
    // The three-header lacing carries a real, non-trivial setup blob.
    assert!(
        codec_private.len() > 32,
        "CodecPrivate carries the 3 Vorbis setup headers"
    );
}

/// Test 4 — Vorbis frame oracle: 45 audio frames; each sample byte length
/// equals the CSV `size`, in order.
#[test]
fn vorbis_frame_oracle() {
    let m = demux(VORBIS_FIXTURE);
    let oracle = load_oracle(VORBIS_ORACLE);
    let aud_oracle = oracle_for(&oracle, "audio");

    assert_eq!(aud_oracle.len(), 45, "oracle sanity: 45 Vorbis frames");
    let aud = &m.tracks[0];
    assert_eq!(aud.samples.len(), 45, "Vorbis sample count");

    for (i, (s, o)) in aud.samples.iter().zip(aud_oracle.iter()).enumerate() {
        assert_eq!(
            s.data.len(),
            o.size,
            "Vorbis sample {i} byte length must equal oracle size"
        );
    }
}

/// Test 5 — regression: the VP9/Opus fixture still demuxes to two tracks with
/// the expected VP9 + Opus configs (the existing WebM path is unaffected).
#[test]
fn vp9_opus_still_demuxes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/webm/vp9_opus.webm"
    );
    let m = demux(path);
    assert_eq!(m.tracks.len(), 2, "VP9 video + Opus audio");
    assert!(
        matches!(m.tracks[0].spec.config, CodecConfig::Vp9 { .. }),
        "track 0 must remain VP9"
    );
    assert!(
        matches!(m.tracks[1].spec.config, CodecConfig::Opus { .. }),
        "track 1 must remain Opus"
    );
    assert_eq!(m.tracks[0].samples.len(), 50, "VP9 frame count unchanged");
    assert_eq!(m.tracks[1].samples.len(), 101, "Opus frame count unchanged");
}
