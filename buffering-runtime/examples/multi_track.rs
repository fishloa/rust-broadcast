//! Audio + video through one controller: the lead is the MINIMUM over the
//! rendered tracks, not video alone.
//!
//! A TS mux sends video ~0.9 s ahead of play time and audio near real time.
//! Two controllers see the same data: one is told only the video position
//! (`on_media`, the old behaviour), the other gets both tracks
//! (`on_track_media`). Run: `cargo run -p buffering-runtime --example multi_track`.

use core::time::Duration;

use broadcast_common::clock33::{PCR_WRAP_27MHZ, pcr_split};
use buffering_runtime::{BufferController, Clock27, ControllerConfig, Event, micros_as_duration};

const STEP_US: u64 = 35_000;
const VIDEO_AHEAD_US: u64 = 900_000;
const TICKS_PER_US: u64 = 27;
const AUDIO: u32 = 1;
const VIDEO: u32 = 2;

fn pos(us: u64) -> Clock27 {
    let ticks = (us * TICKS_PER_US) % PCR_WRAP_27MHZ;
    let (base, ext) = pcr_split(ticks);
    Clock27::from_parts(base, ext).expect("split parts are on the clock")
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
    video_only.on_media(Duration::ZERO, pos(0));
    per_track.register_track(AUDIO);
    per_track.register_track(VIDEO);
    per_track.on_track_media(Duration::ZERO, AUDIO, pos(0));
    per_track.on_track_media(Duration::ZERO, VIDEO, pos(0));
    let mut now = 0;
    while now < 5_000_000 && (vo.is_none() || pt.is_none()) {
        now += STEP_US;
        let at = micros_as_duration(now);
        video_only.on_media(at, pos(now + VIDEO_AHEAD_US));
        per_track.on_track_media(at, AUDIO, pos(now));
        per_track.on_track_media(at, VIDEO, pos(now + VIDEO_AHEAD_US));
        video_only.on_tick(at, None);
        per_track.on_tick(at, None);
        if vo.is_none() && started(&mut video_only) {
            vo = Some(now);
        }
        if pt.is_none() && started(&mut per_track) {
            pt = Some(now);
        }
    }
    println!(
        "start target {} ms; audio buffered when each controller said ready:",
        cfg.policy.initial.as_millis()
    );
    println!("  video only: {:?} ms", vo.map(|t| t / 1000));
    println!(
        "  per track : {:?} ms (limiting track {:?})",
        pt.map(|t| t / 1000),
        per_track.stats().limiting_track
    );
}
