//! Bug audit B: a rejected push must leave the meter exactly as it was.

use broadcast_loudness::{ChannelLayout, LoudnessMeter, TruePeakMeter};

const RATE: u32 = 48_000;

fn tone(i: usize) -> f32 {
    (0.2 * libm_sin(2.0 * core::f64::consts::PI * 997.0 * i as f64 / f64::from(RATE))) as f32
}

fn libm_sin(x: f64) -> f64 {
    x.sin()
}

#[test]
fn rejected_frame_does_not_advance_earlier_channels_filter_state() {
    let mut clean = LoudnessMeter::new(RATE, ChannelLayout::Stereo).unwrap();
    let mut dirty = LoudnessMeter::new(RATE, ChannelLayout::Stereo).unwrap();
    assert!(dirty.push_f32(&[0.9, f32::NAN]).is_err());
    for _ in 0..RATE as usize {
        clean.push_f32(&[0.0, 0.0]).unwrap();
        dirty.push_f32(&[0.0, 0.0]).unwrap();
    }
    clean.finish();
    dirty.finish();
    assert_eq!(clean.max_momentary_lufs(), dirty.max_momentary_lufs());
}

#[test]
fn rejected_stereo_planar_slice_does_not_partially_commit_frames() {
    let n = 3 * RATE as usize;
    let good: Vec<f32> = (0..n).map(tone).collect();
    let mut bad = good.clone();
    bad[1000] = f32::NAN;

    let mut clean = LoudnessMeter::new(RATE, ChannelLayout::Stereo).unwrap();
    let mut dirty = LoudnessMeter::new(RATE, ChannelLayout::Stereo).unwrap();
    assert!(dirty.push_stereo_planar_f32(&bad, &good).is_err());
    assert_eq!(dirty.frame_count(), 0);
    clean.push_stereo_planar_f32(&good, &good).unwrap();
    dirty.push_stereo_planar_f32(&good, &good).unwrap();
    clean.finish();
    dirty.finish();
    assert_eq!(clean.max_momentary_lufs(), dirty.max_momentary_lufs());
    assert_eq!(clean.max_short_term_lufs(), dirty.max_short_term_lufs());
}

#[test]
fn true_peak_rejected_slice_does_not_partially_commit() {
    let mut s: Vec<f32> = (0..64).map(tone).collect();
    s[40] = f32::INFINITY;
    let mut dirty = TruePeakMeter::new();
    assert!(dirty.push_f32_slice(&s).is_err());
    let mut clean = TruePeakMeter::new();
    assert_eq!(clean.finish(), dirty.finish());
}
