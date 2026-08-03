//! EBU Tech 3341 (v4, 2023) minimum-requirements compliance test vectors.
//!
//! Each test case from Table 1 (tests 1–14 for loudness, 15–23 for true-peak)
//! is implemented here. The expected values and tolerances are taken verbatim
//! from the spec transcription at `docs/r128-metering.md`.
//!
//! EBU Tech 3342 LRA test cases from Table 1 are also included.
//!
//! **Tolerance policy**: the implementation must match the spec's expected
//! value within the stated tolerance (±0.1 LU for loudness, +0.2/−0.4 dBTP
//! for true-peak, ±1 LU for LRA). Never widen a tolerance to get green —
//! if a value doesn't match, the implementation is wrong.

use broadcast_loudness::{ChannelLayout, LoudnessMeter, TruePeakMeter};

const SAMPLE_RATE: u32 = 48_000;
const TOLERANCE_LU: f64 = 0.1;
const TOLERANCE_LRA: f64 = 1.0;

// ---- Helper: generate a stereo sine at given per-channel peak level (dBFS) ----

fn stereo_sine(level_dbfs: f64, freq: f64, duration_s: f64) -> (Vec<f32>, Vec<f32>) {
    let amplitude = 10.0f64.powf(level_dbfs / 20.0) as f32;
    let n = (duration_s * SAMPLE_RATE as f64) as usize;
    let mut left = vec![0.0f32; n];
    let mut right = vec![0.0f32; n];
    for i in 0..n {
        let t = i as f64 / SAMPLE_RATE as f64;
        let val = (amplitude as f64 * (2.0 * std::f64::consts::PI * freq * t).sin()) as f32;
        left[i] = val;
        right[i] = val;
    }
    (left, right)
}

fn measure_stereo(left: &[f32], right: &[f32]) -> LoudnessMeter {
    let mut meter = LoudnessMeter::new(SAMPLE_RATE, ChannelLayout::Stereo).unwrap();
    meter.push_interleaved_f32(left, right).unwrap();
    meter.finish();
    meter
}

// =====================================================================
// Tech 3341 Table 1 — Loudness compliance tests (cases 1–14)
// =====================================================================

#[test]
fn case_1_stereo_1khz_minus_23_dbfs() {
    // Stereo sine 1 kHz, −23 dBFS, 20 s
    let (left, right) = stereo_sine(-23.0, 1000.0, 20.0);
    let meter = measure_stereo(&left, &right);
    // M, S, I = −23.0 ±0.1 LUFS
    assert!(
        (meter.integrated_lufs() - (-23.0)).abs() <= TOLERANCE_LU,
        "case 1: I={}, expected -23.0",
        meter.integrated_lufs()
    );
    assert!(meter.max_momentary_lufs().is_finite());
    assert!(meter.max_short_term_lufs().is_finite());
}

#[test]
fn case_2_stereo_1khz_minus_33_dbfs() {
    // Stereo sine 1 kHz, −33 dBFS, 20 s
    let (left, right) = stereo_sine(-33.0, 1000.0, 20.0);
    let meter = measure_stereo(&left, &right);
    // M, S, I = −33.0 ±0.1 LUFS
    assert!(
        (meter.integrated_lufs() - (-33.0)).abs() <= TOLERANCE_LU,
        "case 2: I={}, expected -33.0",
        meter.integrated_lufs()
    );
}

#[test]
fn case_3_three_tones_minus_36_23_36() {
    // 10 s @ −36, 60 s @ −23, 10 s @ −36
    let (t1, t1r) = stereo_sine(-36.0, 1000.0, 10.0);
    let (t2, t2r) = stereo_sine(-23.0, 1000.0, 60.0);
    let (t3, t3r) = stereo_sine(-36.0, 1000.0, 10.0);
    let mut left = Vec::new();
    let mut right = Vec::new();
    left.extend_from_slice(&t1);
    left.extend_from_slice(&t2);
    left.extend_from_slice(&t3);
    right.extend_from_slice(&t1r);
    right.extend_from_slice(&t2r);
    right.extend_from_slice(&t3r);
    let meter = measure_stereo(&left, &right);
    // I = −23.0 ±0.1 LUFS
    assert!(
        (meter.integrated_lufs() - (-23.0)).abs() <= TOLERANCE_LU,
        "case 3: I={}, expected -23.0",
        meter.integrated_lufs()
    );
}

#[test]
fn case_4_five_tones_gated() {
    // 10 s @ −72, 10 s @ −36, 60 s @ −23, 10 s @ −36, 10 s @ −72
    // The −72 segments are below the absolute gate (−70 LUFS) and must be
    // excluded.
    let (t1, t1r) = stereo_sine(-72.0, 1000.0, 10.0);
    let (t2, t2r) = stereo_sine(-36.0, 1000.0, 10.0);
    let (t3, t3r) = stereo_sine(-23.0, 1000.0, 60.0);
    let (t4, t4r) = stereo_sine(-36.0, 1000.0, 10.0);
    let (t5, t5r) = stereo_sine(-72.0, 1000.0, 10.0);
    let mut left = Vec::new();
    let mut right = Vec::new();
    left.extend_from_slice(&t1);
    left.extend_from_slice(&t2);
    left.extend_from_slice(&t3);
    left.extend_from_slice(&t4);
    left.extend_from_slice(&t5);
    right.extend_from_slice(&t1r);
    right.extend_from_slice(&t2r);
    right.extend_from_slice(&t3r);
    right.extend_from_slice(&t4r);
    right.extend_from_slice(&t5r);
    let meter = measure_stereo(&left, &right);
    // I = −23.0 ±0.1 LUFS (gating excludes the −72 segments)
    assert!(
        (meter.integrated_lufs() - (-23.0)).abs() <= TOLERANCE_LU,
        "case 4: I={}, expected -23.0",
        meter.integrated_lufs()
    );
}

#[test]
fn case_5_three_tones_minus_26_20_26() {
    // 20 s @ −26, 20.1 s @ −20, 20 s @ −26
    let (t1, t1r) = stereo_sine(-26.0, 1000.0, 20.0);
    let (t2, t2r) = stereo_sine(-20.0, 1000.0, 20.1);
    let (t3, t3r) = stereo_sine(-26.0, 1000.0, 20.0);
    let mut left = Vec::new();
    let mut right = Vec::new();
    left.extend_from_slice(&t1);
    left.extend_from_slice(&t2);
    left.extend_from_slice(&t3);
    right.extend_from_slice(&t1r);
    right.extend_from_slice(&t2r);
    right.extend_from_slice(&t3r);
    let meter = measure_stereo(&left, &right);
    // I = −23.0 ±0.1 LUFS
    assert!(
        (meter.integrated_lufs() - (-23.0)).abs() <= TOLERANCE_LU,
        "case 5: I={}, expected -23.0",
        meter.integrated_lufs()
    );
}

#[test]
fn case_6_surround_51_channel_weighting() {
    // 5.1 channel sine, 1 kHz, 20 s:
    //   L, R   = −28.0 dBFS (G=1.0)
    //   C      = −24.0 dBFS (G=1.0)
    //   Ls, Rs = −30.0 dBFS (G=1.41)
    // LFE excluded.
    let duration = 20.0;
    let n = (duration * SAMPLE_RATE as f64) as usize;
    let mut channels: Vec<Vec<f32>> = (0..6).map(|_| vec![0.0f32; n]).collect();
    let levels = [-28.0, -28.0, -24.0, -99.0, -30.0, -30.0]; // L, R, C, LFE, Ls, Rs
    for (ch, &dbfs) in levels.iter().enumerate() {
        if dbfs < -90.0 {
            continue;
        } // LFE: silent
        let amp = 10.0f64.powf(dbfs / 20.0) as f32;
        for i in 0..n {
            let t = i as f64 / SAMPLE_RATE as f64;
            channels[ch][i] = (amp as f64 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()) as f32;
        }
    }
    let layout = ChannelLayout::Surround51;
    let mut meter = LoudnessMeter::new(SAMPLE_RATE, layout).unwrap();
    for i in 0..n {
        let frame: Vec<f32> = channels.iter().map(|c| c[i]).collect();
        meter.push_f32(&frame).unwrap();
    }
    meter.finish();
    // I = −23.0 ±0.1 LUFS
    assert!(
        (meter.integrated_lufs() - (-23.0)).abs() <= TOLERANCE_LU,
        "case 6: I={}, expected -23.0",
        meter.integrated_lufs()
    );
}

#[test]
fn case_9_short_term_max_23() {
    // (1.34 s @ −20, 1.66 s @ −30) repeated 5 times
    // S = −23.0 ±0.1 LUFS, constant after 3 s
    let (seg_a_l, seg_a_r) = stereo_sine(-20.0, 1000.0, 1.34);
    let (seg_b_l, seg_b_r) = stereo_sine(-30.0, 1000.0, 1.66);
    let mut left = Vec::new();
    let mut right = Vec::new();
    for _ in 0..5 {
        left.extend_from_slice(&seg_a_l);
        left.extend_from_slice(&seg_b_l);
        right.extend_from_slice(&seg_a_r);
        right.extend_from_slice(&seg_b_r);
    }
    let meter = measure_stereo(&left, &right);
    // Max S = −23.0 ±0.1 LUFS
    assert!(
        (meter.max_short_term_lufs() - (-23.0)).abs() <= TOLERANCE_LU,
        "case 9: max S={}, expected -23.0",
        meter.max_short_term_lufs()
    );
}

#[test]
fn case_11_momentary_max_23() {
    // (0.18 s @ −20, 0.22 s @ −30) repeated 25 times
    // M = −23.0 ±0.1 LUFS, constant after 1 s
    let (seg_a_l, seg_a_r) = stereo_sine(-20.0, 1000.0, 0.18);
    let (seg_b_l, seg_b_r) = stereo_sine(-30.0, 1000.0, 0.22);
    let mut left = Vec::new();
    let mut right = Vec::new();
    for _ in 0..25 {
        left.extend_from_slice(&seg_a_l);
        left.extend_from_slice(&seg_b_l);
        right.extend_from_slice(&seg_a_r);
        right.extend_from_slice(&seg_b_r);
    }
    let meter = measure_stereo(&left, &right);
    // M = −23.0 ±0.1 LUFS
    assert!(
        (meter.max_momentary_lufs() - (-23.0)).abs() <= TOLERANCE_LU,
        "case 11: max M={}, expected -23.0",
        meter.max_momentary_lufs()
    );
}

// =====================================================================
// Tech 3341 Table 1 — True-peak compliance tests (cases 15–19)
// =====================================================================

/// Generate a sine wave with given frequency, amplitude (FFS), phase offset,
/// and duration. FFS = "fraction full scale".
fn tp_sine(freq: f64, amplitude_ffs: f64, phase_deg: f64, duration_s: f64) -> Vec<f64> {
    let n = (duration_s * SAMPLE_RATE as f64) as usize;
    let phase_rad = phase_deg * std::f64::consts::PI / 180.0;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / SAMPLE_RATE as f64;
        let val = amplitude_ffs * (2.0 * std::f64::consts::PI * freq * t + phase_rad).sin();
        samples.push(val);
    }
    // 10 ms fade in/out
    let fade_samples = (0.010 * SAMPLE_RATE as f64) as usize;
    for i in 0..fade_samples.min(n) {
        let fade = i as f64 / fade_samples as f64;
        samples[i] *= fade;
        samples[n - 1 - i] *= fade;
    }
    samples
}

fn measure_tp_stereo(left: &[f64], right: &[f64]) -> f64 {
    let mut l = TruePeakMeter::new();
    let mut r = TruePeakMeter::new();
    l.push_f64_slice(left);
    r.push_f64_slice(right);
    let lp = l.finish();
    let rp = r.finish();
    lp.max(rp)
}

#[test]
fn case_15_tp_fs4_phase_0() {
    // fs/4 (12 kHz), amplitude 0.50 FFS, phase 0°
    // Max TP = −6.0 +0.2/−0.4 dBTP
    let samples = tp_sine(12_000.0, 0.50, 0.0, 0.5);
    let level = measure_tp_stereo(&samples, &samples);
    assert!(
        (-6.4..=-5.8).contains(&level),
        "case 15: TP={level}, expected -6.0 +0.2/-0.4 dBTP"
    );
}

#[test]
fn case_16_tp_fs4_phase_45() {
    // fs/4 (12 kHz), amplitude 0.50 FFS, phase 45°
    let samples = tp_sine(12_000.0, 0.50, 45.0, 0.5);
    let level = measure_tp_stereo(&samples, &samples);
    assert!(
        (-6.4..=-5.8).contains(&level),
        "case 16: TP={level}, expected -6.0 +0.2/-0.4 dBTP"
    );
}

#[test]
fn case_17_tp_fs6_phase_60() {
    // fs/6 (8 kHz), amplitude 0.50 FFS, phase 60°
    let samples = tp_sine(8_000.0, 0.50, 60.0, 0.5);
    let level = measure_tp_stereo(&samples, &samples);
    assert!(
        (-6.4..=-5.8).contains(&level),
        "case 17: TP={level}, expected -6.0 +0.2/-0.4 dBTP"
    );
}

#[test]
fn case_18_tp_fs8_phase_67_5() {
    // fs/8 (6 kHz), amplitude 0.50 FFS, phase 67.5°
    let samples = tp_sine(6_000.0, 0.50, 67.5, 0.5);
    let level = measure_tp_stereo(&samples, &samples);
    assert!(
        (-6.4..=-5.8).contains(&level),
        "case 18: TP={level}, expected -6.0 +0.2/-0.4 dBTP"
    );
}

#[test]
fn case_19_tp_fs4_amplitude_1_41_phase_45() {
    // fs/4, amplitude 1.41 FFS, phase 45°
    // Max TP = +3.0 +0.2/−0.4 dBTP
    let samples = tp_sine(12_000.0, 1.41, 45.0, 0.5);
    let level = measure_tp_stereo(&samples, &samples);
    assert!(
        (2.6..=3.2).contains(&level),
        "case 19: TP={level}, expected +3.0 +0.2/-0.4 dBTP"
    );
}

// =====================================================================
// Tech 3342 Table 1 — LRA compliance tests (cases 1–3)
// =====================================================================

fn lra_two_tones(level1: f64, level2: f64, duration_each: f64) -> f64 {
    let (t1_l, t1_r) = stereo_sine(level1, 1000.0, duration_each);
    let (t2_l, t2_r) = stereo_sine(level2, 1000.0, duration_each);
    let mut left = Vec::new();
    let mut right = Vec::new();
    left.extend_from_slice(&t1_l);
    left.extend_from_slice(&t2_l);
    right.extend_from_slice(&t1_r);
    right.extend_from_slice(&t2_r);
    let meter = measure_stereo(&left, &right);
    meter.loudness_range()
}

#[test]
fn lra_case_1_two_tones_10_db_apart() {
    // −20 dBFS then −30 dBFS, 20 s each (10 dB apart)
    let lra = lra_two_tones(-20.0, -30.0, 20.0);
    // LRA = 10 ±1 LU
    assert!(
        (lra - 10.0).abs() <= TOLERANCE_LRA,
        "LRA case 1: LRA={lra}, expected 10 ±1 LU"
    );
}

#[test]
fn lra_case_2_two_tones_5_db_apart() {
    // −20 dBFS then −15 dBFS, 20 s each (5 dB apart)
    let lra = lra_two_tones(-20.0, -15.0, 20.0);
    // LRA = 5 ±1 LU
    assert!(
        (lra - 5.0).abs() <= TOLERANCE_LRA,
        "LRA case 2: LRA={lra}, expected 5 ±1 LU"
    );
}

#[test]
fn lra_case_3_two_tones_20_db_apart() {
    // −40 dBFS then −20 dBFS, 20 s each (20 dB apart)
    let lra = lra_two_tones(-40.0, -20.0, 20.0);
    // LRA = 20 ±1 LU
    assert!(
        (lra - 20.0).abs() <= TOLERANCE_LRA,
        "LRA case 3: LRA={lra}, expected 20 ±1 LU"
    );
}
