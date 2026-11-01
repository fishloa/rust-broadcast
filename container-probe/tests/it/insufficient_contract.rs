//! Finding 4: the `Insufficient` vs `Unknown` contract at the public API.
//!
//! A truncated prefix of a *real file of a supported format* must report
//! `Insufficient` (read more), never `Unknown` (stop). Before the fix, only the
//! TS prober returned `Insufficient`; every other prober returned `None` on a
//! short buffer, so a 3-byte `.mkv` / a 7-byte `.mp4` / a 15-byte `.mxf` prefix
//! each answered "stop", which is false — reading more could resolve them.
//!
//! Each case feeds a prefix one byte shorter than that prober's minimum, so the
//! *global* `probe` (which returns the smallest `need_at_least` across probers)
//! must come back `Insufficient`, not `Unknown`.
//!
//! (The minimums here mirror `src/`'s per-prober constants; the guards live per
//! prober as `short_prefix_is_insufficient` unit tests where the private
//! constant is visible.)

use container_probe::{Format, Probe};
use std::fs;

fn fixture(rel: &str) -> Vec<u8> {
    fs::read(format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
        .unwrap_or_else(|e| panic!("failed to read fixture {rel}: {e}"))
}

/// Feed the first `min - 1` bytes of `rel` to the public `probe` and assert it
/// is `Insufficient` (never `Unknown`).
fn assert_that_prefix_is_insufficient(rel: &str, min: usize) {
    let data = fixture(rel);
    assert!(
        data.len() >= min,
        "{rel} must be at least {min} bytes to take a {}-byte prefix",
        min - 1
    );
    let prefix = &data[..min - 1];
    match container_probe::probe(prefix) {
        Probe::Insufficient { .. } => {}
        other => panic!(
            "{rel}: a {}-byte prefix of a real file must be Insufficient, got {other:?}",
            min - 1
        ),
    }
}

#[test]
fn ebml_magic_prefix() {
    assert_that_prefix_is_insufficient("fixtures/mkv/h264_aac.mkv", 4);
}

#[test]
fn isobmff_box_header_prefix() {
    assert_that_prefix_is_insufficient("fixtures/mp4/h264_high.mp4", 8);
}

#[test]
fn mxf_key_prefix() {
    assert_that_prefix_is_insufficient("fixtures/mxf/op1a_mpeg2_pcm.mxf", 16);
}

#[test]
fn mpeg_ps_pack_header_prefix() {
    assert_that_prefix_is_insufficient("fixtures/ps/h264_ac3.ps", 14);
}

#[test]
fn flv_header_prefix() {
    assert_that_prefix_is_insufficient("fixtures/flv/av.flv", 9);
}

#[test]
fn riff_wave_prefix() {
    assert_that_prefix_is_insufficient("fixtures/container-probe/pcm_s16le.wav", 12);
}

#[test]
fn ogg_capture_pattern_prefix() {
    assert_that_prefix_is_insufficient("fixtures/container-probe/opus.ogg", 4);
}

#[test]
fn asf_guid_prefix() {
    assert_that_prefix_is_insufficient("fixtures/container-probe/video.asf", 16);
}

#[test]
fn adts_header_prefix() {
    assert_that_prefix_is_insufficient("fixtures/container-probe/aac.adts", 6);
}

#[test]
fn mp3_frame_header_prefix() {
    assert_that_prefix_is_insufficient("fixtures/container-probe/audio.mp3", 4);
}

#[test]
fn annexb_start_code_prefix() {
    assert_that_prefix_is_insufficient("fixtures/container-probe/h264.annexb", 4);
}

/// The empty slice is itself `Insufficient` — an undefined buffer could be
/// anything, so read more. The full file of a supported format is `Identified`,
/// never `Insufficient` (a sanity check that the whole-file path still wins).
#[test]
fn empty_is_insufficient_full_file_identifies() {
    match container_probe::probe(&[]) {
        Probe::Insufficient { need_at_least, .. } => {
            assert!(need_at_least >= 1);
        }
        other => panic!("the empty slice must be Insufficient, got {other:?}"),
    }
    // A full TS file still identifies as its container.
    let data = fixture("fixtures/ts/h264_aac.ts");
    match container_probe::probe(&data) {
        Probe::Identified { format, .. } => assert_eq!(format, Format::MpegTs),
        other => panic!("a full TS fixture must identify, got {other:?}"),
    }
}

/// Finding 4 (extended): a 64-byte prefix of a real elementary-stream file —
/// long enough to read a header, too short for the frame/NAL chain to reach the
/// weak threshold — must also be `Insufficient` (read more), never `Unknown`
/// (stop). The pre-fix probers returned `None` for a chain of length 1..=3,
/// telling a streaming caller reading 64 bytes at a time to STOP on a valid
/// AAC/MP3/H.264 stream.
fn assert_es_prefix_is_insufficient(rel: &str, min: usize) {
    let data = fixture(rel);
    assert!(data.len() >= 64, "{rel} must be at least 64 bytes");
    let prefix = &data[..64];
    match container_probe::probe(prefix) {
        Probe::Insufficient { need_at_least, .. } => {
            assert!(
                need_at_least > prefix.len(),
                "{rel}: need_at_least {need_at_least} must exceed the {} supplied bytes",
                prefix.len()
            );
            assert!(
                need_at_least >= min,
                "{rel}: need_at_least {need_at_least} must be a plausible lower bound (>= {min})"
            );
        }
        other => {
            panic!("{rel}: a 64-byte prefix of a real stream must be Insufficient, got {other:?}")
        }
    }
}

#[test]
fn adts_64_byte_prefix_is_insufficient() {
    // 4 frames at the observed ~274-byte frame size.
    assert_es_prefix_is_insufficient("fixtures/container-probe/aac.adts", 4 * 7);
}

#[test]
fn mp3_64_byte_prefix_is_insufficient() {
    assert_es_prefix_is_insufficient("fixtures/container-probe/audio.mp3", 4 * 4);
}

#[test]
fn annexb_64_byte_prefix_is_insufficient() {
    assert_es_prefix_is_insufficient("fixtures/container-probe/h264.annexb", 4 * 4);
}
