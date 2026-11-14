#![no_main]

use buffering_runtime::{
    BufferController, Clock27, ControllerConfig, Event, Hint, StallMemorySnapshot,
    micros_as_duration,
};
use libfuzzer_sys::fuzz_target;

/// A 27 MHz tick count as a validated playback position, or "not on the clock"
/// (the input is arbitrary, so an out-of-clock value must be skipped, not
/// unwrapped).
fn position(ticks: u64) -> Option<Clock27> {
    Clock27::from_27mhz(ticks % 8_589_934_592 * 300).ok()
}

// Arbitrary operation stream against the controller: must never panic, the
// target stays inside `[min, max]` (the hard cap), and the event
// protocol holds (no Resume without a Pause, no Pause while paused).
fuzz_target!(|data: &[u8]| {
    let cfg = ControllerConfig::default();
    let (min, max) = (cfg.policy.min, cfg.policy.max);
    let seed = (data.first().copied().unwrap_or(0) & 1 == 1)
        .then(|| StallMemorySnapshot::new(micros_as_duration(1_000_000), micros_as_duration(0)));
    let Ok(mut c) = BufferController::new(cfg, seed) else {
        return;
    };
    let (mut playing, mut started, mut paused) = (false, false, false);
    let mut now = 0u64;
    for op in data.chunks_exact(10) {
        let a = u64::from(u32::from_le_bytes([op[2], op[3], op[4], op[5]]));
        let b = u64::from(u32::from_le_bytes([op[6], op[7], op[8], op[9]]));
        // full-range values for some ops, small steps for the rest
        let wide = a << 32 | b;
        now = if op[1] & 1 == 0 {
            now.saturating_add(a % 100_000)
        } else {
            wide
        };
        match op[0] % 10 {
            7 => {
                if let Some(p) = position(wide) {
                    c.on_track_media(micros_as_duration(now), u32::from(op[1] >> 1) % 20, p);
                }
            }
            8 => c.register_track(u32::from(op[1] >> 1) % 20),
            9 => c.unregister_track(u32::from(op[1] >> 1) % 20),
            0 => c.on_chunk(micros_as_duration(now), b as u32, position(wide)),
            1 => c.on_chunk(micros_as_duration(now), b as u32, None),
            2 => {
                if let Some(p) = position(wide) {
                    c.on_media(micros_as_duration(now), p);
                }
            }
            3 => c.on_tick(micros_as_duration(now), position(wide)),
            4 => c.on_tick(micros_as_duration(now), None),
            5 => c.on_hint(
                micros_as_duration(now),
                Hint::from_code(u32::from(op[1] >> 1) % 8).unwrap_or(Hint::SourceEnded),
            ),
            _ => {
                if op[1] & 0x80 != 0 {
                    c.reset(op[1] & 0x40 != 0);
                    playing = false;
                    started = false;
                    paused = false;
                } else {
                    let _ = c.snapshot_memory();
                    let _ = c.stats();
                }
            }
        }
        let t = c.target();
        assert!((min..=max).contains(&t));
        while let Some(e) = c.poll_event() {
            match e {
                Event::StartPlaying { .. } => {
                    assert!(!started);
                    started = true;
                    playing = true;
                }
                Event::Pause { resume_target, .. } => {
                    assert!(playing);
                    assert!(resume_target <= max);
                    playing = false;
                    paused = true;
                }
                Event::Resume { .. } => {
                    assert!(paused && !playing);
                    playing = true;
                    paused = false;
                }
                Event::TargetChanged { new, .. } => assert!(new <= max),
                _ => {}
            }
        }
    }
});
