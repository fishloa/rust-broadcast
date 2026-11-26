//! Streaming-first Phase 5 slice 4 differential gate: the streaming
//! `MatroskaDemux` reproduces the golden track summaries and the golden muxed
//! bytes recorded from the whole-input `WebmDemux`/`MkvMuxStage` on
//! `origin/main`, for every committed Matroska/WebM fixture, seekable and
//! forward-only, and from one-byte, 4 KiB and whole-file feeds.

use test_harness::collect::Collected as Media;
use test_harness::collect::{CollectMode, media_from_matroska};
use transmux::MkvMuxStage;
use transmux::pipeline::CodecConfig;

const FIXTURES: &[&str] = &[
    "mkv/h264-aac.mkv",
    "mkv/hevc-aac.mkv",
    "mkv/vorbis.webm",
    "mkv/vp8-opus.webm",
    "mkv/vp9-opus.mkv",
    "mkv/vp9-opus.webm",
];

struct Golden {
    tracks: Vec<String>,
    mkv: String,
}

fn golden(fixture: &str) -> Golden {
    let name = fixture.rsplit('/').next().unwrap_or(fixture);
    let text = test_fixtures::read_to_string(&format!("mkv/golden/{name}.txt"));
    let mut tracks = Vec::new();
    let mut mkv = String::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("mkv ") {
            mkv = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("track ") {
            tracks.push(rest.to_string());
        }
    }
    Golden { tracks, mkv }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

fn codec_id(config: &CodecConfig) -> &'static str {
    match config {
        CodecConfig::Vp9 { .. } => "V_VP9",
        CodecConfig::Vp8 { .. } => "V_VP8",
        CodecConfig::Avc { .. } => "V_MPEG4/ISO/AVC",
        CodecConfig::Hevc { .. } => "V_MPEGH/ISO/HEVC",
        CodecConfig::Opus { .. } => "A_OPUS",
        CodecConfig::Vorbis { .. } => "A_VORBIS",
        CodecConfig::Aac { .. } => "A_AAC",
        other => panic!("fixture codec {other:?} is not in the golden set"),
    }
}

fn track_line(track: &test_harness::collect::CollectedTrack) -> String {
    let mut all = Vec::new();
    for sample in &track.samples {
        all.extend_from_slice(&sample.data);
    }
    let first = track
        .samples
        .first()
        .and_then(|sample| sample.timing.and_then(|timing| timing.pts()))
        .unwrap_or(0);
    let last = track
        .samples
        .last()
        .and_then(|sample| sample.timing.and_then(|timing| timing.pts()))
        .unwrap_or(0);
    format!(
        "{} {} samples={} first={} last={} fnv={:016x}",
        track.spec.track_id.value(),
        codec_id(&track.spec.config),
        track.samples.len(),
        first,
        last,
        fnv1a(&all)
    )
}

fn track_lines(media: &Media) -> Vec<String> {
    media.tracks.iter().map(track_line).collect()
}

#[test]
fn every_fixture_reproduces_the_recorded_track_summaries() {
    for fixture in FIXTURES {
        let want = golden(fixture);
        let bytes = test_fixtures::read(fixture);
        let seekable = media_from_matroska(&bytes, CollectMode::Seekable).expect(fixture);
        assert_eq!(track_lines(&seekable), want.tracks, "{fixture}: seekable");
        for chunk in [1usize, 4096, usize::MAX] {
            let forward = media_from_matroska(&bytes, CollectMode::Sequential { chunk })
                .unwrap_or_else(|error| panic!("{fixture}: sequential chunk {chunk}: {error:?}"));
            assert_eq!(
                track_lines(&forward),
                want.tracks,
                "{fixture}: sequential chunk {chunk}"
            );
        }
    }
}

#[test]
fn every_fixture_reproduces_the_recorded_mux_bytes() {
    for fixture in FIXTURES {
        let want = golden(fixture);
        let bytes = test_fixtures::read(fixture);
        let media = media_from_matroska(&bytes, CollectMode::Seekable).expect(fixture);
        let out = crate::mux_bytes::mux_bytes(
            &mut MkvMuxStage::new(core::time::Duration::from_millis(5_000)),
            &media,
        );
        assert_eq!(
            format!("len={} fnv={:016x}", out.len(), fnv1a(&out)),
            want.mkv,
            "{fixture}: mux output"
        );
    }
}
