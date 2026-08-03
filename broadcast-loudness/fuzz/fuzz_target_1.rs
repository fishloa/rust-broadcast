#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Fuzz the LoudnessMeter with arbitrary interleaved f32 samples.
    // Convert arbitrary bytes to f32 frames and feed them to the meter.
    if data.len() < 8 {
        return;
    }

    // Interpret bytes as interleaved stereo f32 (4 bytes per sample)
    let num_samples = data.len() / 8; // 2 channels × 4 bytes
    let mut left: Vec<f32> = Vec::with_capacity(num_samples);
    let mut right: Vec<f32> = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let offset = i * 8;
        let l_bytes: [u8; 4] = data[offset..offset + 4].try_into().unwrap();
        let r_bytes: [u8; 4] = data[offset + 4..offset + 8].try_into().unwrap();
        let l = f32::from_le_bytes(l_bytes);
        let r = f32::from_le_bytes(r_bytes);

        // Clamp to sane range to avoid NaN/inf poisoning
        if l.is_finite() && r.is_finite() {
            left.push(l);
            right.push(r);
        }
    }

    if left.is_empty() {
        return;
    }

    // Also test TruePeakMeter with the same data
    let mut tp = broadcast_loudness::TruePeakMeter::new();
    for &l in &left {
        tp.push_f32(l);
    }
    let _ = tp.finish();

    // LoudnessMeter: only 48 kHz is valid
    let mut meter = match broadcast_loudness::LoudnessMeter::new(
        48_000,
        broadcast_loudness::ChannelLayout::Stereo,
    ) {
        Ok(m) => m,
        Err(_) => return,
    };
    let _ = meter.push_interleaved_f32(&left, &right);
    meter.finish();
    let _ = meter.integrated_lufs();
    let _ = meter.loudness_range();
    let _ = meter.max_momentary_lufs();
    let _ = meter.max_short_term_lufs();
});
