//! Issue #1072 (audit r14-LOUD-C2): both meters must run in memory bounded
//! by the *window* sizes (a few seconds of audio), never by the measurement
//! duration. The whole test thread runs under a 8 MiB live-byte cap
//! (`test_alloc::ThreadCapped`): the pre-fix meters stored one `f64` per
//! sample frame (and per true-peak sample), which at the volume below needs
//! well over 100 MiB, so an allocation over the cap returns null and the
//! process aborts. Results are cross-checked against closed-form values.

use broadcast_loudness::{ChannelLayout, LoudnessMeter, TruePeakMeter};
use test_alloc::ThreadCapped;

#[global_allocator]
static CAP: ThreadCapped<{ 8 * 1024 * 1024 }> = ThreadCapped::new();

const RATE: u32 = 8_000;
/// 5 minutes of stereo at 8 kHz = 2.4 M frames (19 MB as one f64 each, over the
/// cap); 30 minutes under `RUST_BROADCAST_SOAK`.
fn frames() -> usize {
    let minutes = if std::env::var_os("RUST_BROADCAST_SOAK").is_some() {
        30
    } else {
        5
    };
    RATE as usize * 60 * minutes
}

#[test]
fn loudness_meter_is_bounded_and_correct_over_a_long_run() {
    let mut m = LoudnessMeter::new(RATE, ChannelLayout::Stereo).unwrap();
    // 1 kHz-class sine at 0.1 amplitude on both channels, fed in 1-frame pushes
    // from a precomputed one-second table (so the test itself allocates nothing).
    let table: Vec<f32> = (0..RATE)
        .map(|i| {
            0.1 * (2.0 * std::f64::consts::PI * 1000.0 * f64::from(i) / f64::from(RATE)).sin()
                as f32
        })
        .collect();
    for n in 0..frames() {
        let s = table[n % table.len()];
        m.push_f32(&[s, s]).unwrap();
    }
    m.finish();
    // A stereo 1 kHz sine of amplitude A: mean square A^2/2 per channel,
    // two channels at G=1 -> A^2; LKFS = -0.691 + 10 log10(A^2 * K(1kHz)^2)
    // with K(997 Hz) = +0.691 dB, so -20.0 LUFS for A = 0.1.
    let expect = -20.0; // K-weighting is +0.691 dB at ~1 kHz, cancelling the -0.691 offset
    assert!(
        (m.integrated_lufs() - expect).abs() < 0.1,
        "{}",
        m.integrated_lufs()
    );
    assert!(m.loudness_range() < 0.1);
}

#[test]
fn true_peak_meter_is_bounded_over_a_long_run() {
    let mut tp = TruePeakMeter::new();
    for n in 0..frames() {
        let s = if n % 4 == 0 { 0.5f32 } else { -0.25 };
        tp.push_f32(s).unwrap();
    }
    // O(1) live read, no re-processing of history.
    let live = tp.current_level();
    assert!(live >= -6.1, "{live}");
    assert!(tp.finish() >= live);
}
