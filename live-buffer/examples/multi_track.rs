//! Audio + video through one controller: the lead is the MINIMUM over the
//! rendered tracks, not video alone.
//!
//! A TS mux sends video ~0.9 s ahead of play time and audio near real time.
//! Two controllers see the same data: one is told only the video position
//! (`on_media`, the old behaviour), the other gets both tracks
//! (`on_track_media`). Run: `cargo run -p live-buffer --example multi_track`.

use live_buffer::{BufferController, ControllerConfig, Event, WRAP_27MHZ};

const STEP_US: u64 = 35_000;
const VIDEO_AHEAD_US: u64 = 900_000;
const TICKS_PER_US: u64 = 27;
const AUDIO: u32 = 1;
const VIDEO: u32 = 2;

fn pos(us: u64) -> u64 {
    (us * TICKS_PER_US) % WRAP_27MHZ
}

fn started(c: &mut BufferController) -> bool {
    let mut hit = false;
    while let Some(ev) = c.poll_event() {
        hit |= matches!(ev, Event::StartPlaying { .. });
    }
    hit
}

fn main() {
    let cfg = ControllerConfig::default();
    let mut video_only = BufferController::new(cfg, None).expect("default config is valid");
    let mut per_track = BufferController::new(cfg, None).expect("default config is valid");
    let (mut vo, mut pt) = (None, None);
    video_only.on_media(0, pos(0));
    per_track.register_track(AUDIO);
    per_track.register_track(VIDEO);
    per_track.on_track_media(0, AUDIO, pos(0));
    per_track.on_track_media(0, VIDEO, pos(0));
    let mut now = 0;
    while now < 5_000_000 && (vo.is_none() || pt.is_none()) {
        now += STEP_US;
        video_only.on_media(now, pos(now + VIDEO_AHEAD_US));
        per_track.on_track_media(now, AUDIO, pos(now));
        per_track.on_track_media(now, VIDEO, pos(now + VIDEO_AHEAD_US));
        video_only.on_tick(now, None);
        per_track.on_tick(now, None);
        if vo.is_none() && started(&mut video_only) {
            vo = Some(now);
        }
        if pt.is_none() && started(&mut per_track) {
            pt = Some(now);
        }
    }
    println!(
        "start target {} ms; audio buffered when each controller said ready:",
        cfg.policy.initial_us / 1000
    );
    println!("  video only: {:?} ms", vo.map(|t| t / 1000));
    println!(
        "  per track : {:?} ms (limiting track {:?})",
        pt.map(|t| t / 1000),
        per_track.stats().limiting_track
    );
}
