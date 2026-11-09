//! Bug audit A (2026-11-09): reproductions that fail on main fed51ad27.

use broadcast_common::clock33::PCR_HZ;
use buffering_runtime::{BufferController, ControllerConfig, Event, Ticks27};

const TICKS_PER_US: u64 = PCR_HZ / 1_000_000;
const STEP_US: u64 = 40_000;
const VIDEO: u32 = 1;
const AUDIO: u32 = 2;

fn t27(us: u64) -> Ticks27 {
    us * TICKS_PER_US
}

struct Sim {
    c: BufferController,
    now: u64,
    clock: Ticks27,
    running: bool,
    t_set: u64,
    log: Vec<(u64, Event)>,
}

impl Sim {
    fn new() -> Self {
        Self {
            c: BufferController::new(ControllerConfig::default(), None).unwrap(),
            now: 0,
            clock: 0,
            running: false,
            t_set: 0,
            log: vec![],
        }
    }

    fn pos(&self) -> Ticks27 {
        if self.running {
            self.clock + (self.now - self.t_set) * TICKS_PER_US
        } else {
            self.clock
        }
    }

    fn tick(&mut self) {
        let p = self.running.then(|| self.pos());
        self.c.on_tick(self.now, p.or(Some(self.clock)));
        while let Some(e) = self.c.poll_event() {
            match e {
                Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                    self.clock = media_27mhz;
                    self.t_set = self.now;
                    self.running = true;
                }
                Event::Pause { hold_at_27mhz, .. } => {
                    self.clock = hold_at_27mhz;
                    self.running = false;
                }
                _ => {}
            }
            self.log.push((self.now, e));
        }
    }

    fn starts(&self) -> usize {
        self.log
            .iter()
            .filter(|(_, e)| matches!(e, Event::StartPlaying { .. }))
            .count()
    }

    fn pauses(&self) -> usize {
        self.log
            .iter()
            .filter(|(_, e)| matches!(e, Event::Pause { .. }))
            .count()
    }

    fn rebaselines(&self) -> usize {
        self.log
            .iter()
            .filter(|(_, e)| matches!(e, Event::Rebaselined))
            .count()
    }
}

/// A track registered while Playing (an audio switch, or a PMT update adding
/// the rendered audio) has no end yet, so `effective()` is `None` and
/// `on_tick` returns before the starvation check: a delivery stall in that
/// window lets the renderer clock run past the data with no `Pause` ever.
#[test]
fn track_registered_while_playing_must_not_disable_starvation_detection() {
    let mut s = Sim::new();
    let media_ahead = 500_000;
    while s.now < 4_000_000 {
        s.c.on_chunk(s.now, 1316, Some(t27(s.now + media_ahead)));
        s.c.on_track_media(s.now, VIDEO, t27(s.now));
        s.tick();
        s.now += STEP_US;
    }
    assert_eq!(s.starts(), 1, "precondition: playing on video");
    let last_end_us = s.now - STEP_US;
    s.c.register_track(AUDIO);
    // global delivery stall: nothing arrives anywhere for 2.5 s, the renderer keeps ticking
    let stall_end = s.now + 2_500_000;
    while s.now < stall_end {
        s.tick();
        s.now += STEP_US;
    }
    let pos_us = s.pos() / TICKS_PER_US;
    assert!(
        s.pauses() >= 1,
        "no Pause although the renderer clock ({pos_us} us) ran {} ms past the last media end ({last_end_us} us); events: {:?}",
        (pos_us.saturating_sub(last_end_us)) / 1000,
        s.log
    );
}

/// PCR-less source (RTSP/RTMP), two unregistered tracks, video ahead of audio
/// (the case `on_track_media` docs describe). In PCR-less mode the estimator
/// is fed the effective (minimum) end; when audio's first report lowers that
/// minimum by more than `discontinuity_us`, the estimator reports a
/// Discontinuity and `rebaseline` drops every unregistered track. The next
/// chunk repeats it, so the stream rebaselines on every chunk and never starts.
#[test]
fn pcr_less_second_track_joining_behind_is_not_a_discontinuity() {
    let mut s = Sim::new();
    let video_ahead = 1_200_000;
    let audio_from = 400_000;
    while s.now < 6_000_000 {
        s.c.on_chunk(s.now, 1316, None);
        s.c.on_track_media(s.now, VIDEO, t27(s.now + video_ahead));
        if s.now >= audio_from {
            s.c.on_track_media(s.now, AUDIO, t27(s.now));
        }
        s.tick();
        s.now += STEP_US;
    }
    let st = s.c.stats();
    assert!(
        s.rebaselines() == 0 && s.starts() == 1,
        "rebaselines {} discontinuities {} starts {}",
        s.rebaselines(),
        st.discontinuities,
        s.starts()
    );
}

/// A restored stall memory is anchored at the first clock reading as
/// `at = now - age` (`MaxHold::anchor`, saturating). When the new
/// controller's clock starts below the snapshot's age (a host clock that
/// starts at 0 per session) the age is clamped to `now`, so the peak is
/// treated as that much younger and is held and decayed far too late.
#[test]
fn restored_stall_memory_keeps_its_age_when_the_clock_starts_small() {
    use buffering_runtime::StallMemorySnapshot;
    let peak = 1_000_000;
    let age = 60_000_000;
    let seed = StallMemorySnapshot::new(peak, age);
    let mut c = BufferController::new(ControllerConfig::default(), Some(seed)).unwrap();
    // anchored at the first clock reading (1 s) as 60 s old; 30 s later it is
    // 90 s old: 30 s into the 60 s decay
    c.on_chunk(1_000_000, 1316, Some(0));
    c.on_chunk(31_000_000, 1316, Some(t27(30_000_000)));
    let held = c.stats().est.stall_peak_us;
    assert_eq!(
        held,
        peak / 2,
        "stall memory 90 s after its peak should have decayed to half ({} us), reads {held} us",
        peak / 2
    );
}

/// PCR-less: once the track feeding the estimator is removed, the next track
/// takes over with a fresh baseline; its media clock 1.2 s behind is not a
/// discontinuity, so playback goes on without a rebaseline.
#[test]
fn pcr_less_estimator_track_removed_hands_over_without_a_discontinuity() {
    let mut s = Sim::new();
    let video_ahead = 1_200_000;
    while s.now < 4_000_000 {
        s.c.on_chunk(s.now, 1316, None);
        s.c.on_track_media(s.now, VIDEO, t27(s.now + video_ahead));
        s.c.on_track_media(s.now, AUDIO, t27(s.now));
        s.tick();
        s.now += STEP_US;
    }
    assert_eq!(s.starts(), 1, "precondition: playing");
    s.c.unregister_track(VIDEO);
    while s.now < 8_000_000 {
        s.c.on_chunk(s.now, 1316, None);
        s.c.on_track_media(s.now, AUDIO, t27(s.now));
        s.tick();
        s.now += STEP_US;
    }
    assert_eq!(s.rebaselines(), 0, "events: {:?}", s.log);
    assert_eq!(s.c.stats().discontinuities, 0);
}
