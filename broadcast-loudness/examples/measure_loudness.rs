//! Measure loudness of a stereo 1 kHz sine wave at -23 LUFS.
//!
//! This example demonstrates the basic loudness measurement API:
//! create a meter, push planar f32 samples, and query results.

use broadcast_loudness::{ChannelLayout, LoudnessMeter};

fn main() {
    let sample_rate = 48_000u32;
    let layout = ChannelLayout::Stereo;
    let duration = 5.0; // 5 seconds
    let n = (duration * sample_rate as f64) as usize;

    // Generate a stereo 1 kHz sine wave at -23 LUFS (per-channel peak)
    let amplitude = 10.0f64.powf(-23.0 / 20.0) as f32;
    let mut left = vec![0.0f32; n];
    let mut right = vec![0.0f32; n];
    for i in 0..n {
        let t = i as f64 / sample_rate as f64;
        let val = (amplitude as f64 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()) as f32;
        left[i] = val;
        right[i] = val;
    }

    // Feed to the meter (interleaved API — one slice per channel)
    let mut meter = LoudnessMeter::new(sample_rate, layout).unwrap();
    meter.push_interleaved_f32(&left, &right).unwrap();
    meter.finish();

    println!("Integrated loudness: {:.1} LUFS", meter.integrated_lufs());
    println!(
        "Max momentary:       {:.1} LUFS",
        meter.max_momentary_lufs()
    );
    println!(
        "Max short-term:      {:.1} LUFS",
        meter.max_short_term_lufs()
    );
    println!("Loudness range:      {:.1} LU", meter.loudness_range());
    println!("Duration:            {:.1} s", meter.duration_seconds());
}
