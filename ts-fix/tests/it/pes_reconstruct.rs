//! PES access-unit reconstruction tests on a real H.264+AAC broadcast capture.
//!
//! The fixture `fixtures/ts/h264_aac.ts` is known to contain:
//! - H.264 video PES on PID 0x0100  (75 PUSI / 75 access units)
//! - AAC audio PES on PID 0x0101    (14 PUSI / 14 access units)
//! - Both streams carry PTS on every access unit.
//!
//! All tests here are **byte-assertion** tests (not codec-parsing tests) —
//! they verify framing counts, PTS presence, and the identity passthrough
//! property.

use mpeg_ts::pid::Pid;
use std::{fs, path::PathBuf};

/// Path to the shared H.264+AAC fixture in the workspace root.
fn fixture_path() -> PathBuf {
    test_fixtures::path("ts/h264_aac.ts")
}

fn load_fixture() -> Vec<u8> {
    let path = fixture_path();
    fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {path:?}: {e}"))
}

// ── PES reconstruction counts and structure ─────────────────────────────────

#[test]
fn reconstruct_pes_video_pid_0100() {
    let ts = load_fixture();
    let aus =
        ts_fix::pes::reconstruct_access_units(&ts, &[Pid::new(0x0100), Pid::new(0x0101)]).unwrap();

    let video: Vec<_> = aus.iter().filter(|au| au.pid == Pid::new(0x0100)).collect();
    let audio: Vec<_> = aus.iter().filter(|au| au.pid == Pid::new(0x0101)).collect();

    assert_eq!(video.len(), 75, "expected 75 access units on PID 0x0100");
    assert_eq!(audio.len(), 14, "expected 14 access units on PID 0x0101");
}

#[test]
fn all_video_aus_have_pts() {
    let ts = load_fixture();
    let aus = ts_fix::pes::reconstruct_access_units(&ts, &[Pid::new(0x0100)]).unwrap();
    assert!(!aus.is_empty(), "expected at least one AU on PID 0x0100");
    for au in &aus {
        assert!(
            au.pts.is_some(),
            "PID 0x0100 AU missing PTS: pts={:?}",
            au.pts
        );
    }
}

#[test]
fn all_audio_aus_have_pts() {
    let ts = load_fixture();
    let aus = ts_fix::pes::reconstruct_access_units(&ts, &[Pid::new(0x0101)]).unwrap();
    assert!(!aus.is_empty(), "expected at least one AU on PID 0x0101");
    for au in &aus {
        assert!(
            au.pts.is_some(),
            "PID 0x0101 AU missing PTS: pts={:?}",
            au.pts
        );
    }
}

#[test]
fn every_au_starts_with_pes_start_code() {
    let ts = load_fixture();
    let aus =
        ts_fix::pes::reconstruct_access_units(&ts, &[Pid::new(0x0100), Pid::new(0x0101)]).unwrap();
    assert!(!aus.is_empty(), "expected at least one access unit");
    for au in &aus {
        assert!(
            au.data.len() >= 4,
            "AU data too short ({} bytes) for PES start code",
            au.data.len()
        );
        // PES start code prefix is 00 00 01, followed by stream_id.
        assert_eq!(
            au.data[0..3],
            [0x00, 0x00, 0x01],
            "PID {:#06x} AU does not start with PES start code",
            au.pid
        );
        // stream_id byte exists (byte at index 3).
        assert!(au.data[3] != 0, "PID {:#06x} AU has zero stream_id", au.pid);
    }
}

#[test]
fn video_dts_non_decreasing() {
    let ts = load_fixture();
    let aus = ts_fix::pes::reconstruct_access_units(&ts, &[Pid::new(0x0100)]).unwrap();
    let mut prev_dts: Option<u64> = None;
    for au in &aus {
        if let Some(dts) = au.dts {
            if let Some(prev) = prev_dts {
                assert!(dts >= prev, "DTS regressed: {dts} < {prev}");
            }
            prev_dts = Some(dts);
        }
    }
}

// ── Identity passthrough (byte-identical) ───────────────────────────────────

#[test]
fn identity_passthrough_byte_identical() {
    let input = load_fixture();

    // Sanity: fixture must be a non-empty multiple of 188.
    assert!(!input.is_empty(), "fixture is empty");
    assert_eq!(
        input.len() % 188,
        0,
        "fixture length {} is not a multiple of 188",
        input.len()
    );

    let mut engine = ts_fix::TsFix::builder()
        .build()
        .expect("identity build should not fail");

    let mut output: Vec<u8> = Vec::with_capacity(input.len());

    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| {
                output.extend_from_slice(pkt);
            })
            .expect("valid 188-byte packet from fixture");
    }

    engine.finish(|pkt| {
        output.extend_from_slice(pkt);
    });

    // Output is byte-identical to input.
    assert_eq!(
        output.len(),
        input.len(),
        "output length {} differs from input length {}",
        output.len(),
        input.len()
    );
    assert_eq!(output, input, "identity engine output differs from input");
}

/// A buffer that is not a whole number of 188-byte packets is a typed error,
/// not a panic (review of the signalling/captions wave, item 12).
#[test]
fn unaligned_input_is_a_typed_error() {
    for len in [1usize, 187, 189, 188 * 3 + 1] {
        let ts = vec![0x47u8; len];
        match ts_fix::pes::reconstruct_access_units(&ts, &[Pid::new(0x0100)]) {
            Err(ts_fix::Error::UnalignedInput { len: got }) => assert_eq!(got, len),
            other => panic!("len {len}: expected UnalignedInput, got {other:?}"),
        }
    }
    // Aligned (including empty) input is accepted.
    assert!(
        ts_fix::pes::reconstruct_access_units(&[], &[Pid::new(0x0100)])
            .unwrap()
            .is_empty()
    );
}
