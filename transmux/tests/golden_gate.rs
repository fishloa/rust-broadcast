//! Player-validated golden gate (issue #569).
//!
//! Every other transmux test proves round-trip symmetry against *our own*
//! parsers — self-referential. This harness closes that gap: it runs a real
//! fixture through the same `transmux::cli::run_bytes` code path the shipped
//! `transmux` binary uses, then hands the produced artefact to an
//! **independent** decoder (`ffprobe`) and asserts it: (a) parses without a
//! decode error and (b) reports the track count / codec / dimensions /
//! sample-rate the source fixture itself reports — not a value we hardcode.
//!
//! Cases (all driven from `fixtures/ts/h264_aac.ts`, a real 2-track H.264 +
//! AAC MPEG-2 TS capture, ISO/IEC 13818-1):
//!   1. TS → CMAF/fMP4 (`CmafMux`)
//!   2. TS → progressive MP4 (`ProgressiveMux`)
//!   3. TS → classic TS-HLS: a media segment **and** the `.m3u8` playlist
//!   4. TS → DASH MPD: `ffprobe`'s own `dash` demuxer on the `.mpd` if the
//!      local build has it (rare — needs libxml2), else a structural MPD
//!      check plus `ffprobe` on the referenced CMAF segments.
//!
//! `ffprobe` availability (and specific demuxers, e.g. `hls`/`dash`) is
//! probed once and every case skips cleanly — printing why — when the tool a
//! case needs is absent, so `cargo test` stays green without ffmpeg
//! installed. A case that *does* run must genuinely assert against parsed
//! JSON fields; "ffprobe exited 0" alone is not a pass.
//!
//! WebVTT is not covered here: transmux's CLI has no WebVTT `OutputFormat`
//! today (the #568 CEA-608/708→WebVTT work landed in the sibling
//! `timed-metadata` crate, not as a transmux mux spoke), so there is nothing
//! for this harness to produce yet.

// This harness drives `transmux::cli::run_bytes`, which only exists under the
// `cli` feature; gate the whole test file so `--no-default-features` builds
// (e.g. the MSRV job) don't fail to compile on the missing `transmux::cli`.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use transmux::cli::{Opts, Output, OutputFormat, run_bytes};

// ── Fixture + scratch-dir plumbing ──────────────────────────────────────────

fn ts_fixture() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    std::fs::read(path).expect("h264_aac.ts fixture must exist")
}

/// A fresh, empty scratch directory under the workspace `target/` (already
/// gitignored) for one test case's output artefacts.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/golden-gate-tmp")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create golden-gate scratch dir");
    dir
}

// ── External-tool availability gate ─────────────────────────────────────────

/// True if `ffprobe -version` runs successfully on `PATH`.
fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// True if `ffprobe -demuxers` lists a demuxer whose comma-separated short
/// name list contains exactly `name` (e.g. `"dash"`, `"hls"`).
fn ffprobe_has_demuxer(name: &str) -> bool {
    let Ok(out) = Command::new("ffprobe").arg("-demuxers").output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().any(|line| {
        // Format: " <flags>  <name1>,<name2>,...  <description>"
        line.split_whitespace()
            .nth(1)
            .is_some_and(|names| names.split(',').any(|n| n == name))
    })
}

/// Skip this test cleanly with a printed reason. `cargo test` without ffmpeg
/// installed must stay green — never a false failure for a missing tool.
macro_rules! skip_unless {
    ($cond:expr, $why:expr) => {
        if !$cond {
            eprintln!("SKIP golden_gate: {}", $why);
            return;
        }
    };
}

// ── ffprobe JSON helpers ─────────────────────────────────────────────────────

/// Run `ffprobe -v error -show_format -show_streams -of json <path>` and
/// parse the result. Panics (fails the test) on a non-zero exit or malformed
/// JSON — at this point the tool is known to be present, so a failure here is
/// a genuine defect in the produced media, not a missing-tool skip.
fn ffprobe_json(path: &Path) -> Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .expect("spawn ffprobe");
    assert!(
        out.status.success(),
        "ffprobe rejected {}: stderr={}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "ffprobe -v error printed diagnostics for {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("ffprobe -of json must produce valid JSON")
}

/// The single stream object in `probe["streams"]` whose `codec_type` matches,
/// or panic — every case here expects exactly one video and one audio stream.
fn stream<'a>(probe: &'a Value, codec_type: &str) -> &'a Value {
    probe["streams"]
        .as_array()
        .expect("streams array")
        .iter()
        .find(|s| s["codec_type"] == codec_type)
        .unwrap_or_else(|| panic!("no {codec_type} stream in ffprobe output: {probe}"))
}

fn nb_streams(probe: &Value) -> u64 {
    probe["format"]["nb_streams"]
        .as_u64()
        .expect("format.nb_streams")
}

/// Assert `out_probe`'s video/audio identification matches `src_probe`'s —
/// codec name, dimensions, sample rate/channels — the source fixture's own
/// ffprobe identification is the oracle, never a hardcoded literal.
fn assert_matches_source(src_probe: &Value, out_probe: &Value, what: &str) {
    assert_eq!(
        nb_streams(out_probe),
        nb_streams(src_probe),
        "{what}: stream count must match the source fixture"
    );

    let src_v = stream(src_probe, "video");
    let out_v = stream(out_probe, "video");
    assert_eq!(
        out_v["codec_name"], src_v["codec_name"],
        "{what}: video codec_name must match source"
    );
    assert_eq!(
        out_v["width"], src_v["width"],
        "{what}: video width must match source"
    );
    assert_eq!(
        out_v["height"], src_v["height"],
        "{what}: video height must match source"
    );

    let src_a = stream(src_probe, "audio");
    let out_a = stream(out_probe, "audio");
    assert_eq!(
        out_a["codec_name"], src_a["codec_name"],
        "{what}: audio codec_name must match source"
    );
    assert_eq!(
        out_a["sample_rate"], src_a["sample_rate"],
        "{what}: audio sample_rate must match source"
    );
    assert_eq!(
        out_a["channels"], src_a["channels"],
        "{what}: audio channel count must match source"
    );
}

// ── Case 1: TS → CMAF/fMP4 ───────────────────────────────────────────────────

#[test]
fn ts_to_cmaf_ffprobe_validated() {
    skip_unless!(ffprobe_available(), "ffprobe not on PATH");

    let ts = ts_fixture();
    let src_probe = {
        let dir = scratch_dir("cmaf-src");
        let path = dir.join("src.ts");
        std::fs::write(&path, &ts).unwrap();
        ffprobe_json(&path)
    };

    let out = run_bytes(
        &ts,
        &Opts {
            format: OutputFormat::Cmaf,
            ..Opts::default()
        },
    )
    .expect("run_bytes TS -> CMAF");
    let bytes = match out {
        Output::Bytes(b) => b,
        Output::Manifest { .. } => panic!("CMAF must produce a single binary artefact"),
    };
    assert!(!bytes.is_empty(), "CMAF output must not be empty");

    let dir = scratch_dir("cmaf-out");
    let path = dir.join("out.mp4");
    std::fs::write(&path, &bytes).unwrap();

    let out_probe = ffprobe_json(&path);
    assert_matches_source(&src_probe, &out_probe, "TS->CMAF");
}

// ── Case 2: TS → progressive MP4 ─────────────────────────────────────────────

#[test]
fn ts_to_progressive_mp4_ffprobe_validated() {
    skip_unless!(ffprobe_available(), "ffprobe not on PATH");

    let ts = ts_fixture();
    let src_probe = {
        let dir = scratch_dir("prog-src");
        let path = dir.join("src.ts");
        std::fs::write(&path, &ts).unwrap();
        ffprobe_json(&path)
    };

    let out = run_bytes(
        &ts,
        &Opts {
            format: OutputFormat::Progressive,
            ..Opts::default()
        },
    )
    .expect("run_bytes TS -> progressive");
    let bytes = match out {
        Output::Bytes(b) => b,
        Output::Manifest { .. } => panic!("progressive MP4 must produce a single binary artefact"),
    };
    assert!(
        !bytes.is_empty(),
        "progressive MP4 output must not be empty"
    );

    let dir = scratch_dir("prog-out");
    let path = dir.join("out.mp4");
    std::fs::write(&path, &bytes).unwrap();

    let out_probe = ffprobe_json(&path);
    assert_matches_source(&src_probe, &out_probe, "TS->progressive");
}

// ── Case 3: TS → classic TS-HLS (segment + playlist) ────────────────────────

#[test]
fn ts_to_ts_hls_segment_and_playlist_ffprobe_validated() {
    skip_unless!(ffprobe_available(), "ffprobe not on PATH");

    let ts = ts_fixture();
    let src_probe = {
        let dir = scratch_dir("tshls-src");
        let path = dir.join("src.ts");
        std::fs::write(&path, &ts).unwrap();
        ffprobe_json(&path)
    };

    // 1s target segments against a 3s fixture: enough cuts to exercise the
    // playlist window/discontinuity machinery, not just a single segment.
    let out = run_bytes(
        &ts,
        &Opts {
            format: OutputFormat::TsHls,
            segment_duration: 1,
            ..Opts::default()
        },
    )
    .expect("run_bytes TS -> TS-HLS");
    let (playlist, segments) = match out {
        Output::Manifest { text, segments } => (text, segments),
        Output::Bytes(_) => panic!("TS-HLS must produce a manifest + segments"),
    };
    assert!(
        playlist.contains("#EXTM3U"),
        "playlist must be a valid M3U8 (missing #EXTM3U)"
    );
    assert!(
        !segments.is_empty(),
        "TS-HLS must emit at least one segment"
    );

    let dir = scratch_dir("tshls-out");
    let playlist_path = dir.join("out.m3u8");
    std::fs::write(&playlist_path, &playlist).unwrap();
    for (name, bytes) in &segments {
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    // (a) A single media segment, in isolation, is a well-formed MPEG-TS
    // carrying the same codec identity as the source.
    let seg_probe = ffprobe_json(&dir.join(&segments[0].0));
    assert_matches_source(&src_probe, &seg_probe, "TS->TS-HLS segment");

    // (b) The playlist itself, resolved through ffprobe's `hls` demuxer
    // (reads every referenced segment), reports the same identity.
    if ffprobe_has_demuxer("hls") {
        let pl_probe = ffprobe_json(&playlist_path);
        assert_matches_source(&src_probe, &pl_probe, "TS->TS-HLS playlist");
    } else {
        eprintln!(
            "SKIP golden_gate: local ffprobe has no `hls` demuxer; \
             validated the segment only, not the playlist end-to-end"
        );
    }
}

// ── Case 4: TS → DASH MPD ────────────────────────────────────────────────────

#[test]
fn ts_to_dash_mpd_validated() {
    skip_unless!(ffprobe_available(), "ffprobe not on PATH");

    let ts = ts_fixture();
    let src_probe = {
        let dir = scratch_dir("dash-src");
        let path = dir.join("src.ts");
        std::fs::write(&path, &ts).unwrap();
        ffprobe_json(&path)
    };

    let out = run_bytes(
        &ts,
        &Opts {
            format: OutputFormat::Dash,
            ..Opts::default()
        },
    )
    .expect("run_bytes TS -> DASH");
    let (mpd, segments) = match out {
        Output::Manifest { text, segments } => (text, segments),
        Output::Bytes(_) => panic!("DASH must produce an MPD manifest + segments"),
    };
    assert!(
        !segments.is_empty(),
        "DASH must emit referenced CMAF segments"
    );

    let dir = scratch_dir("dash-out");
    let mpd_path = dir.join("out.mpd");
    std::fs::write(&mpd_path, &mpd).unwrap();
    for (name, bytes) in &segments {
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    // Structural MPD check — always run, independent of ffprobe's dash
    // demuxer support. Cheap substring checks on a hand-rolled XML writer's
    // output are appropriate here (a full XML parser would be overkill); this
    // is a genuine bite: an empty/truncated/malformed MPD fails every check.
    assert!(mpd.contains("<MPD"), "MPD must have a root <MPD> element");
    assert!(
        mpd.contains("</MPD>"),
        "MPD must be a complete, closed document"
    );
    let adaptation_sets = mpd.matches("<AdaptationSet").count();
    assert_eq!(
        adaptation_sets, 2,
        "MPD must carry exactly 2 AdaptationSets (video + audio): got {adaptation_sets}"
    );
    assert!(
        mpd.contains("video/mp4"),
        "MPD must carry a video/mp4 AdaptationSet mimeType"
    );
    assert!(
        mpd.contains("audio/mp4"),
        "MPD must carry an audio/mp4 AdaptationSet mimeType"
    );
    assert!(
        mpd.contains("<SegmentTemplate"),
        "MPD must address segments via SegmentTemplate"
    );

    // ffprobe validation of the referenced CMAF segments: each artefact
    // ffprobe's mov/mp4 demuxer, so this catches a genuinely broken segment
    // even when the MPD text alone would look fine.
    for (name, _) in &segments {
        let probe = ffprobe_json(&dir.join(name));
        // Each segment is the full per-track CMAF stream this CLI path
        // reuses for both `init-*` and `chunk-*` names (see
        // `transmux::cli::package`'s `OutputFormat::Dash` arm) — i.e. video
        // and audio are muxed together in one file, same as the CMAF case.
        assert_matches_source(&src_probe, &probe, &format!("TS->DASH segment {name}"));
    }

    // ffprobe's own `dash` demuxer (needs libxml2; not every ffmpeg build has
    // it) is the strongest oracle when available: resolve the whole MPD.
    if ffprobe_has_demuxer("dash") {
        let mpd_probe = ffprobe_json(&mpd_path);
        assert_matches_source(&src_probe, &mpd_probe, "TS->DASH MPD");
    } else {
        eprintln!(
            "SKIP golden_gate: local ffprobe has no `dash` demuxer; \
             validated the structural MPD + referenced segments only"
        );
    }
}

// ── Deliberate-regression sanity: the gate must actually bite ───────────────

/// Proves `assert_matches_source` is not vacuous: a mutated (corrupted)
/// output artefact must fail ffprobe validation, not silently pass. This is
/// the harness's own self-test — it does not touch the library.
#[test]
fn mutated_cmaf_output_fails_the_gate() {
    skip_unless!(ffprobe_available(), "ffprobe not on PATH");

    let ts = ts_fixture();
    let out = run_bytes(
        &ts,
        &Opts {
            format: OutputFormat::Cmaf,
            ..Opts::default()
        },
    )
    .expect("run_bytes TS -> CMAF");
    let mut bytes = match out {
        Output::Bytes(b) => b,
        Output::Manifest { .. } => panic!("CMAF must produce a single binary artefact"),
    };
    assert!(
        bytes.len() > 64,
        "fixture output too small to mutate meaningfully"
    );

    // Truncate the artefact hard enough to destroy its box structure (cut it
    // to well inside the first box, before any full ISOBMFF box is present).
    bytes.truncate(16);

    let dir = scratch_dir("cmaf-mutated");
    let path = dir.join("broken.mp4");
    std::fs::write(&path, &bytes).unwrap();

    let probe_ok = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(&path)
        .output()
        .expect("spawn ffprobe")
        .status
        .success();
    assert!(
        !probe_ok,
        "a truncated/corrupted CMAF artefact must NOT pass ffprobe — \
         if it does, assert_matches_source's ffprobe oracle is not biting"
    );
}
