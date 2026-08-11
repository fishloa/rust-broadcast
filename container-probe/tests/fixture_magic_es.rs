//! WP3 magic + elementary-stream prober fixtures — exact verdicts (never a
//! disjunction), plus the cross-prober suppression regression tests.
//!
//! Confidences below are what the crate actually measures on the real fixtures,
//! not assumed: FLV/WAV/Ogg/ASF are magic-only `STRONG` (192); the three
//! elementary-stream fixtures each chain enough frames/NALs for `LATTICE_STRONG`
//! (144). Fixture paths are workspace-relative (joined to `CARGO_MANIFEST_DIR`).

use container_probe::{Confidence, Format, Probe};
use std::fs;

fn fixture(rel: &str) -> Vec<u8> {
    fs::read(format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
        .unwrap_or_else(|e| panic!("failed to read fixture {rel}: {e}"))
}

/// Assert a file is `Identified` as exactly `format`.
fn assert_identified(rel: &str, format: Format) {
    let p = container_probe::probe(&fixture(rel));
    assert!(
        matches!(p, Probe::Identified { format: f, .. } if f == format),
        "{rel}: expected {format:?}, got {p:?}"
    );
}

/// Assert a file is `Identified` as exactly `format` at `confidence`.
fn assert_identified_as(rel: &str, format: Format, confidence: Confidence) {
    let p = container_probe::probe(&fixture(rel));
    assert!(
        matches!(
            p,
            Probe::Identified { format: f, confidence: c, .. } if f == format && c == confidence
        ),
        "{rel}: expected {format:?} @ {confidence:?}, got {p:?}"
    );
}

// ---------------------------------------------------------------------------
// Simple magic probers (STRONG, 192).
// ---------------------------------------------------------------------------

#[test]
fn flv_is_flv() {
    assert_identified_as("fixtures/flv/av.flv", Format::Flv, Confidence::STRONG);
}

#[test]
fn wav_is_wav() {
    assert_identified_as(
        "fixtures/container-probe/pcm_s16le.wav",
        Format::Wav,
        Confidence::STRONG,
    );
}

#[test]
fn ogg_is_ogg() {
    assert_identified_as(
        "fixtures/container-probe/opus.ogg",
        Format::Ogg,
        Confidence::STRONG,
    );
}

#[test]
fn asf_is_asf() {
    assert_identified_as(
        "fixtures/container-probe/video.asf",
        Format::Asf,
        Confidence::STRONG,
    );
}

// ---------------------------------------------------------------------------
// Elementary streams (LATTICE_STRONG, 144, by length chaining).
// ---------------------------------------------------------------------------

#[test]
fn aac_adts_is_adts() {
    assert_identified_as(
        "fixtures/container-probe/aac.adts",
        Format::AdtsAac,
        Confidence::LATTICE_STRONG,
    );
}

#[test]
fn mp3_is_mp3() {
    assert_identified_as(
        "fixtures/container-probe/audio.mp3",
        Format::Mp3,
        Confidence::LATTICE_STRONG,
    );
}

#[test]
fn annexb_is_annexb() {
    assert_identified_as(
        "fixtures/container-probe/h264.annexb",
        Format::AnnexB,
        Confidence::LATTICE_STRONG,
    );
}

// ---------------------------------------------------------------------------
// Cross-prober suppression — these containers carry many ES syncwords (see the
// brief's measurements) and MUST identify as their container, never as an ES.
// ---------------------------------------------------------------------------

#[test]
fn suppression_ts_carries_adts_and_mp3() {
    assert_identified("fixtures/ts/h264_aac.ts", Format::MpegTs);
}

#[test]
fn suppression_big_ts() {
    assert_identified("fixtures/ts/france2.ts", Format::MpegTs);
}

#[test]
fn suppression_mp4() {
    assert_identified("fixtures/mp4/h264_high.mp4", Format::Isobmff);
}

#[test]
fn suppression_mkv() {
    assert_identified("fixtures/mkv/h264_aac.mkv", Format::Matroska);
}

/// MPEG-PS begins `00 00 01 BA` — a start code whose NAL header `0xBA` has
/// `forbidden_zero_bit` set, so it must NOT be Annex B. It is MpegPs (STRUCTURAL).
#[test]
fn suppression_ps_starts_with_start_code_but_is_not_annexb() {
    assert_identified("fixtures/ps/h264_ac3.ps", Format::MpegPs);
}

#[test]
fn suppression_mxf_carries_annexb_start_codes() {
    assert_identified("fixtures/mxf/op1a_mpeg2_pcm.mxf", Format::Mxf);
}

/// The assembly-level half of the "an ES prober fires directly on a container's
/// payload" proof. This buffer is **synthetic** — a TS lattice followed by a
/// 32-frame ADTS run — built inline because it exists to exercise the harness's
/// suppression rule, not to claim any real file does this (no corpus container
/// has an ADTS chain above 1). The ES-prober-itself half lives in the
/// `#[cfg(test)]` module inside `src/adts.rs`, which can call the prober
/// directly; here we can only use the public API, so we assert the assembled
/// outcome.
///
/// # Mutation proof (mutation #5 — disable `suppress_elementary_streams`)
///
/// Without suppression this buffer's TS lattice (MpegTs 144) and ADTS chain
/// (AdtsAac 144) tie within `TIE_THRESHOLD`, producing `Ambiguous` — the exact
/// failure the rule exists to prevent. Observed failure with suppression
/// disabled:
/// ```
/// synthetic TS+ADTS must be MpegTs under suppression, got
/// Ambiguous { candidates: [AdtsAac 144, MpegTs 144] }
/// ```
#[test]
fn suppression_synthetic_ts_carrying_adts() {
    let data = synthetic_ts_carrying_adts(32);
    let p = container_probe::probe(&data);
    assert!(
        matches!(
            p,
            Probe::Identified {
                format: Format::MpegTs,
                ..
            }
        ),
        "synthetic TS+ADTS must be MpegTs under suppression, got {p:?}"
    );
}

/// Build the synthetic TS lattice + ADTS frames buffer.
///
/// Intentionally a byte-copy of the `#[cfg(test)]` helper in `src/adts.rs`
/// (this external test cannot reach that crate-private helper, so the
/// duplication is unavoidable without widening a `pub(crate)` item). Drift
/// between the two is caught by `adts::tests::synthetic_ts_carrying_adts_layout_is_pinned`,
/// which fixes the exact byte layout both must produce.
fn synthetic_ts_carrying_adts(frame_count: usize) -> Vec<u8> {
    let aac_len = frame_count * 274usize;
    let packets = std::cmp::max(12, aac_len.div_ceil(188) * 2);
    let mut v = vec![0u8; packets * 188 + frame_count * 274];
    for i in 0..packets {
        v[i * 188] = 0x47; // TS sync at every packet start
    }
    // ADTS frames, each chaining via its own frame_length (274).
    let base = packets * 188;
    for k in 0..frame_count {
        let o = base + k * 274;
        let mut f = vec![0u8; 274];
        f[0] = 0xFF;
        f[1] = 0xF1;
        f[3] = 0; // 274 < 0x800, so bit 11 of frame_length is clear
        f[4] = ((274u16 >> 3) & 0xFF) as u8;
        f[5] = ((274u16 & 0x07) as u8) << 5;
        v[o..o + 274].copy_from_slice(&f);
    }
    v
}
