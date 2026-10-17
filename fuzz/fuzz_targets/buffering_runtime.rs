#![no_main]

use libfuzzer_sys::fuzz_target;
use buffering_runtime::{BufferController, ControllerConfig, Event, Hint, StallMemorySnapshot};

/// Arbitrary operation stream against the controller: must never panic, the
/// target stays inside `[min_us, max_us]` (the hard cap), and the event
/// protocol holds (no Resume without a Pause, no Pause while paused).
fuzz_target!(|data: &[u8]| {
    let cfg = ControllerConfig::default();
    let (min, max) = (cfg.policy.min_us, cfg.policy.max_us);
    let seed = (data.first().copied().unwrap_or(0) & 1 == 1).then(|| StallMemorySnapshot::new(1_000_000, 0));
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
        now = if op[1] & 1 == 0 { now.saturating_add(a % 100_000) } else { wide };
        match op[0] % 10 {
            7 => c.on_track_media(now, u32::from(op[1] >> 1) % 20, wide),
            8 => c.register_track(u32::from(op[1] >> 1) % 20),
            9 => c.unregister_track(u32::from(op[1] >> 1) % 20),
            0 => c.on_chunk(now, b as u32, Some(wide)),
            1 => c.on_chunk(now, b as u32, None),
            2 => c.on_media(now, wide),
            3 => c.on_tick(now, Some(wide)),
            4 => c.on_tick(now, None),
            5 => c.on_hint(
                now,
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
        let t = c.target_us();
        assert!((min..=max).contains(&t));
        while let Some(e) = c.poll_event() {
            match e {
                Event::StartPlaying { .. } => {
                    assert!(!started);
                    started = true;
                    playing = true;
                }
                Event::Pause { resume_target_us, .. } => {
                    assert!(playing);
                    assert!(resume_target_us <= max);
                    playing = false;
                    paused = true;
                }
                Event::Resume { .. } => {
                    assert!(paused && !playing);
                    playing = true;
                    paused = false;
                }
                Event::TargetChanged { new_us, .. } => assert!(new_us <= max),
                _ => {}
            }
        }
    }
});
