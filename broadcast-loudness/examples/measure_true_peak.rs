//! Measure true-peak level of a stereo signal.
//!
//! Demonstrates the `TruePeakMeter` with a pure 1 kHz sine wave at -6 dBFS.

use broadcast_loudness::TruePeakMeter;

fn main() {
    let sample_rate = 48_000.0;
    let duration = 0.5; // 500 ms
    let n = (duration * sample_rate) as usize;
    let freq = 1000.0;

    // Create one meter per channel (true-peak is per-channel)
    let mut left_meter = TruePeakMeter::new();
    let mut right_meter = TruePeakMeter::new();

    // Generate stereo 1 kHz sine at -6 dBFS
    let amplitude = 10.0f64.powf(-6.0 / 20.0); // ~0.5
    for i in 0..n {
        let t = i as f64 / sample_rate;
        let val = amplitude * (2.0 * std::f64::consts::PI * freq * t).sin();
        left_meter.push_f64(val).unwrap();
        right_meter.push_f64(val).unwrap();
    }

    let left_tp = left_meter.finish();
    let right_tp = right_meter.finish();
    let max_tp = left_tp.max(right_tp);

    println!("Left  true-peak:  {:.1} dBTP", left_tp);
    println!("Right true-peak:  {:.1} dBTP", right_tp);
    println!("Max   true-peak:  {:.1} dBTP", max_tp);
    // Expected: approximately -6.0 dBTP for a -6 dBFS sine
    println!("(Expected ~-6.0 dBTP for a -6 dBFS sine wave)");
}
