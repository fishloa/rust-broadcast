//! WebM demuxer integration tests (issue #471) — oracle-driven.
//!
//! The fixture `fixtures/webm/vp9_opus.webm` is a real WebM (VP9 video + Opus
//! audio, ffmpeg-produced, no lacing). Its per-frame ffprobe oracle lives in
//! `fixtures/webm/vp9_opus.packets.csv`:
//!
//! ```text
//! codec_type,stream_index,pts,dts,duration,size,keyframe(K=1)
//! ```
//!
//! `pts`/`dts`/`duration` are in **milliseconds** (ffprobe stream time_base
//! 1/1000). The `size` column is the per-frame coded byte length — the strong
//! bite: wrong VINT/block parsing yields wrong sizes.
//!
//! The demuxer emits an IR whose timescale is milliseconds
//! ([`transmux::webm_demux::IR_TIMESCALE`] = 1000), so a sample's reconstructed
//! PTS (cumulative sample durations from the track's first block) is directly in
//! the oracle's units. For **video** the reconstructed PTS equals the oracle PTS
//! exactly. For **audio**, ffprobe shifts the presentation time back by the Opus
//! codec delay (pre-skip 312 samples @ 48 kHz ≈ 7 ms — see the fixture's
//! `initial_padding`), so `recon_pts == oracle_pts + AUDIO_CODEC_DELAY_MS`; the
//! test documents and applies that constant.

use broadcast_common::Package;
use transmux::pipeline::CodecConfig;
use transmux::webm_demux::WebmDemux;
use transmux::{parse_box, CmafMux, Media};

/// Path to the committed WebM fixture, relative to this crate's manifest dir.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/webm/vp9_opus.webm"
);
/// Path to the ffprobe per-frame oracle CSV.
const ORACLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/webm/vp9_opus.packets.csv"
);

/// Opus codec delay ffprobe applies to audio presentation times, in ms.
/// (pre-skip 312 samples @ 48 kHz = 6.5 ms, rounded up to 7 ms.)
const AUDIO_CODEC_DELAY_MS: i64 = 7;

/// One ffprobe oracle row.
#[derive(Debug)]
struct OracleRow {
    codec_type: String,
    pts: i64,
    size: usize,
    keyframe: bool,
}

fn load_oracle() -> Vec<OracleRow> {
    let text = std::fs::read_to_string(ORACLE).expect("read oracle csv");
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        rows.push(OracleRow {
            codec_type: f[0].to_string(),
            pts: f[2].trim().parse().unwrap(),
            size: f[5].trim().parse().unwrap(),
            keyframe: f[6].trim() == "1",
        });
    }
    rows
}

fn oracle_for<'a>(rows: &'a [OracleRow], codec_type: &str) -> Vec<&'a OracleRow> {
    rows.iter().filter(|r| r.codec_type == codec_type).collect()
}

fn demux() -> Media {
    let bytes = std::fs::read(FIXTURE).expect("read webm fixture");
    let mut d = WebmDemux::new();
    d.demux(&bytes).expect("demux webm")
}

/// Test 1 — stream enumeration: exactly 2 tracks, track 0 = VP9 video,
/// track 1 = Opus audio.
#[test]
fn enumerates_two_tracks_vp9_and_opus() {
    let m = demux();
    assert_eq!(m.tracks.len(), 2, "expected exactly 2 tracks");
    assert!(
        matches!(m.tracks[0].spec.config, CodecConfig::Vp9 { .. }),
        "track 0 must be VP9 video, got {:?}",
        m.tracks[0].spec.config
    );
    assert!(
        matches!(m.tracks[1].spec.config, CodecConfig::Opus { .. }),
        "track 1 must be Opus audio, got {:?}",
        m.tracks[1].spec.config
    );
}

/// Test 2 — frame counts + per-frame size oracle. Each demuxed sample's coded
/// byte length must equal the oracle `size` column, in order (a wrong block /
/// VINT parse yields wrong sizes).
#[test]
fn frame_counts_and_sizes_match_oracle() {
    let m = demux();
    let oracle = load_oracle();
    let vid_oracle = oracle_for(&oracle, "video");
    let aud_oracle = oracle_for(&oracle, "audio");

    assert_eq!(vid_oracle.len(), 50, "oracle sanity: 50 video frames");
    assert_eq!(aud_oracle.len(), 101, "oracle sanity: 101 audio frames");

    let vid = &m.tracks[0];
    let aud = &m.tracks[1];
    assert_eq!(vid.samples.len(), 50, "video sample count");
    assert_eq!(aud.samples.len(), 101, "audio sample count");

    for (i, (s, o)) in vid.samples.iter().zip(vid_oracle.iter()).enumerate() {
        assert_eq!(
            s.data.len(),
            o.size,
            "video sample {i} byte length must equal oracle size"
        );
    }
    for (i, (s, o)) in aud.samples.iter().zip(aud_oracle.iter()).enumerate() {
        assert_eq!(
            s.data.len(),
            o.size,
            "audio sample {i} byte length must equal oracle size"
        );
    }
}

/// Test 3 — timestamp + keyframe oracle. Reconstructed PTS (cumulative sample
/// durations from the track's first block, ms) matches the oracle, and video
/// keyframe flags match the oracle keyframe column. Audio is all-sync and its
/// oracle PTS is codec-delay-shifted (see [`AUDIO_CODEC_DELAY_MS`]).
#[test]
fn timestamps_and_keyframes_match_oracle() {
    let m = demux();
    let oracle = load_oracle();
    let vid_oracle = oracle_for(&oracle, "video");
    let aud_oracle = oracle_for(&oracle, "audio");

    // Video: IR timescale is ms, first block PTS is 0 → recon_pts == oracle pts.
    let vid = &m.tracks[0];
    let mut acc = 0i64;
    for (i, s) in vid.samples.iter().enumerate() {
        assert_eq!(
            acc, vid_oracle[i].pts,
            "video sample {i} reconstructed PTS must equal oracle"
        );
        assert_eq!(
            s.is_sync, vid_oracle[i].keyframe,
            "video sample {i} keyframe flag must equal oracle"
        );
        acc += s.duration as i64;
    }

    // Audio: recon_pts == oracle_pts + codec delay; every sample is sync.
    let aud = &m.tracks[1];
    let mut acc = 0i64;
    for (i, s) in aud.samples.iter().enumerate() {
        assert_eq!(
            acc,
            aud_oracle[i].pts + AUDIO_CODEC_DELAY_MS,
            "audio sample {i} reconstructed PTS must equal oracle + codec delay"
        );
        assert!(s.is_sync, "audio sample {i} must be a sync sample");
        acc += s.duration as i64;
    }
}

/// Test 4 — Opus config from OpusHead. The built `dOps` carries the channel
/// count + pre-skip parsed from the CodecPrivate `OpusHead` (mono here). The
/// magic was actually parsed: pre-skip 312 is a real OpusHead value that only
/// appears if the header was read, not defaulted (a default `OpusSpecificBox`
/// has pre-skip 0).
#[test]
fn opus_config_from_opus_head() {
    let m = demux();
    let CodecConfig::Opus {
        config,
        channel_count,
        sample_rate,
        ..
    } = &m.tracks[1].spec.config
    else {
        panic!("track 1 is not Opus");
    };
    // Channels: the OpusHead channel count matches the fixture's Audio/Channels (1).
    assert_eq!(*channel_count, 1, "Opus channel count from Audio element");
    assert_eq!(
        config.output_channel_count, 1,
        "dOps OutputChannelCount from OpusHead"
    );
    // Pre-skip is a real OpusHead value (312), proving the header was parsed.
    assert_eq!(
        config.pre_skip, 312,
        "dOps PreSkip from OpusHead (not defaulted)"
    );
    assert_eq!(*sample_rate, 48_000, "Opus playback rate is always 48 kHz");
    assert_eq!(config.version, 1, "OpusHead version");
}

/// Test 5 — output path works: the demuxed IR muxes to fMP4 with a `vp09` video
/// sample entry carrying a `vpcC` box and an `Opus` audio sample entry carrying a
/// `dOps` box (proves the IR configs are complete enough to mux).
#[test]
fn demuxed_ir_muxes_to_fmp4_with_vp09_and_opus() {
    let m = demux();
    let mut mux = CmafMux::default();
    let fmp4 = mux.package(&m).expect("mux demuxed IR to CMAF");

    // Scan for the fourccs anywhere in the emitted bytes (they only appear if the
    // sample entries + config boxes were built from the IR configs).
    assert!(
        contains(&fmp4, b"vp09"),
        "fMP4 must carry a vp09 sample entry"
    );
    assert!(contains(&fmp4, b"vpcC"), "fMP4 must carry a vpcC box");
    assert!(
        contains(&fmp4, b"Opus"),
        "fMP4 must carry an Opus sample entry"
    );
    assert!(contains(&fmp4, b"dOps"), "fMP4 must carry a dOps box");

    // Structurally: the leading box parses (it is a real ISOBMFF file, not noise).
    let (bx, _) = parse_box(&fmp4).expect("first box parses");
    assert_eq!(&bx.header.box_type.0, b"ftyp", "fMP4 begins with ftyp");
}

/// Find `needle` anywhere in `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
