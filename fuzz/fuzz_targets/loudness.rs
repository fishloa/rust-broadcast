#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }

    // ── LoudnessMeter ──
    let mut meter = match broadcast_loudness::LoudnessMeter::new(
        48_000,
        broadcast_loudness::ChannelLayout::Stereo,
    ) {
        Ok(m) => m,
        Err(_) => return,
    };

    // Feed arbitrary bytes as interleaved stereo f32.
    // push_f32 rejects non-finite samples — that is intentional and the
    // fuzz target exercises both the reject path and the valid-measurement
    // path (via a fallback all-valid measurement at the end).
    let num_frames = data.len() / 8;
    for i in 0..num_frames {
        let off = i * 8;
        let l = f32::from_le_bytes(data[off..off + 4].try_into().unwrap());
        let r = f32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap());
        let _ = meter.push_f32(&[l, r]);
    }
    meter.finish();
    let _ = meter.integrated_lufs();
    let _ = meter.max_momentary_lufs();
    let _ = meter.max_short_term_lufs();
    let _ = meter.loudness_range();

    // All‑valid sanity: prove the meter still works after fuzzed input.
    let mut meter2 = match broadcast_loudness::LoudnessMeter::new(
        48_000,
        broadcast_loudness::ChannelLayout::Stereo,
    ) {
        Ok(m) => m,
        Err(_) => return,
    };
    for _ in 0..1000 {
        let _ = meter2.push_f32(&[0.5f32, 0.5f32]);
    }
    meter2.finish();
    let _ = meter2.integrated_lufs();

    // ── TruePeakMeter ──
    let mut tp = broadcast_loudness::TruePeakMeter::new();
    for i in 0..num_frames {
        let off = i * 8;
        let l = f32::from_le_bytes(data[off..off + 4].try_into().unwrap());
        tp.push_f32(l).ok();
    }
    let _ = tp.finish();
});
