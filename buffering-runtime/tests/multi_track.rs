//! Per-track media ends: the lead is judged on the MINIMUM over the rendered
//! tracks, not on video alone (Apple TV client bug: TS muxes send video ~1 s
//! ahead of play time and audio near real time; a video-only lead made every
//! audio buffer arrive after its play time).
//!
//! Synthetic scenario, 35 ms steps, media time == arrival time:
//! video end = t + `VIDEO_AHEAD`, audio end = t. Positions start near the
//! 27 MHz wrap so every comparison also crosses it.

use broadcast_common::clock33::PCR_HZ;
use buffering_runtime::{
    BufferController, ControllerConfig, DEFAULT_TRACK, Event, EventRecord, MAX_TRACKS, State,
    Ticks27, WRAP_27MHZ,
};

const AUDIO: u32 = 1;
const VIDEO: u32 = 2;
const STEP_US: u64 = 35_000;
const VIDEO_AHEAD_US: u64 = 900_000;
const TICKS_PER_US: u64 = PCR_HZ / 1_000_000;
/// 10 s before the 27 MHz wrap.
const BASE: Ticks27 = WRAP_27MHZ - 10 * PCR_HZ;

fn at(base: Ticks27, offset_us: u64) -> Ticks27 {
    (base + offset_us * TICKS_PER_US) % WRAP_27MHZ
}

/// Renderer + controller driven in lock step.
struct Sim {
    c: BufferController,
    now: u64,
    /// Media time of the data delivered so far, in microseconds.
    t_us: u64,
    clock: Ticks27,
    running: bool,
    t_set: u64,
    log: Vec<(u64, Event)>,
}

impl Sim {
    fn new(cfg: ControllerConfig) -> Self {
        Self {
            c: BufferController::new(cfg, None).unwrap(),
            now: 1_000_000,
            t_us: 0,
            clock: BASE,
            running: false,
            t_set: 0,
            log: vec![],
        }
    }
    fn pos(&self) -> Ticks27 {
        if self.running {
            at(self.clock, self.now - self.t_set)
        } else {
            self.clock
        }
    }
    fn drain(&mut self) {
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
    fn tick(&mut self) {
        let p = Some(self.pos());
        self.c.on_tick(self.now, p);
        self.drain();
    }
    fn first(&self) -> Vec<(u32, Ticks27)> {
        vec![(AUDIO, BASE), (VIDEO, BASE)]
    }
    fn starts(&self) -> Vec<(u64, u64)> {
        self.log
            .iter()
            .filter_map(|(t, e)| match e {
                Event::StartPlaying { lead_us, .. } => Some((*t, *lead_us)),
                _ => None,
            })
            .collect()
    }
    fn resumes(&self) -> Vec<u64> {
        self.log
            .iter()
            .filter_map(|(t, e)| matches!(e, Event::Resume { .. }).then_some(*t))
            .collect()
    }
    fn pauses(&self) -> Vec<(u64, Option<u32>)> {
        self.log
            .iter()
            .filter_map(|(t, e)| match e {
                Event::Pause { track, .. } => Some((*t, *track)),
                _ => None,
            })
            .collect()
    }
    /// One 35 ms step delivering the listed tracks.
    fn step(&mut self, audio: bool, video: bool) {
        self.now += STEP_US;
        self.t_us += STEP_US;
        if audio {
            self.c.on_track_media(self.now, AUDIO, at(BASE, self.t_us));
        }
        if video {
            self.c
                .on_track_media(self.now, VIDEO, at(BASE, self.t_us + VIDEO_AHEAD_US));
        }
        self.tick();
    }
}

fn cfg() -> ControllerConfig {
    ControllerConfig::default()
}

fn started_sim() -> Sim {
    let mut s = Sim::new(cfg());
    for (t, e) in s.first() {
        s.c.on_track_media(s.now, t, e);
    }
    while s.starts().is_empty() {
        s.step(true, true);
        assert!(s.t_us < 10_000_000, "never started");
    }
    s
}

/// The bug: video alone says "ready" while audio has only buffered
/// `t`; the per-track controller waits for audio.
#[test]
fn video_only_lead_starts_early_min_over_tracks_waits_for_audio() {
    let start_us = cfg().policy.initial_us;

    // Video-only (the old behaviour): `on_media` fed the video end.
    let mut video_only = Sim::new(cfg());
    video_only.c.on_media(video_only.now, BASE);
    // Per-track.
    let mut both = Sim::new(cfg());
    for (t, e) in both.first() {
        both.c.on_track_media(both.now, t, e);
    }
    while video_only.starts().is_empty() {
        video_only.now += STEP_US;
        video_only.t_us += STEP_US;
        video_only
            .c
            .on_media(video_only.now, at(BASE, video_only.t_us + VIDEO_AHEAD_US));
        video_only.tick();
        assert!(video_only.t_us < 10_000_000);
    }
    while both.starts().is_empty() {
        both.step(true, true);
        assert!(both.t_us < 10_000_000);
    }
    // The video-only controller started with only `start - ahead` of audio.
    let (_, vo_lead) = video_only.starts()[0];
    let audio_lead_at_vo_start = video_only.t_us;
    assert!(audio_lead_at_vo_start + VIDEO_AHEAD_US >= start_us);
    assert!(
        audio_lead_at_vo_start < start_us,
        "audio had only {audio_lead_at_vo_start} us when video-only said ready"
    );
    assert!(vo_lead >= start_us);
    // The min-over-tracks controller started only once AUDIO had a full start lead.
    let (t_both, both_lead) = both.starts()[0];
    assert!(both_lead >= start_us);
    assert!(
        both.t_us >= start_us,
        "StartPlaying with audio at {} us",
        both.t_us
    );
    assert!(t_both > video_only.starts()[0].0);
    // And its reported lead is the audio lead, not the video lead.
    assert!(both_lead < start_us + 2 * STEP_US);
}

/// A starved audio track (video still arriving) pauses playback, is named,
/// and Resume is gated on audio, not on the plentiful video.
#[test]
fn starved_track_is_named_and_resume_waits_for_it() {
    let mut s = started_sim();
    assert_eq!(s.c.state(), State::Playing);
    assert_eq!(s.c.stats().limiting_track, Some(AUDIO));
    // Audio stops (video continues, well inside the silence timeout).
    let stopped = s.now;
    while s.pauses().is_empty() {
        s.step(false, true);
        assert!(s.now - stopped < 2_900_000, "never starved");
    }
    let (_, track) = s.pauses()[0];
    assert_eq!(track, Some(AUDIO));
    assert_eq!(s.c.state(), State::Starved);
    assert_eq!(s.c.stats().limiting_track, Some(AUDIO));
    let rec = s
        .log
        .iter()
        .find_map(|(_, e)| matches!(e, Event::Pause { .. }).then(|| e.to_record()))
        .unwrap();
    assert_eq!((rec.track, rec.track_valid), (AUDIO, 1));
    // Video alone keeps arriving: still no Resume.
    let before = s.now;
    while s.now - before < 600_000 {
        s.step(false, true);
    }
    assert!(s.resumes().is_empty(), "resumed on video alone");
    // Audio returns: Resume only once ITS lead reaches the resume target.
    while s.resumes().is_empty() {
        s.step(true, true);
        assert!(s.now - before < 6_000_000, "never resumed");
    }
    assert_eq!(s.c.state(), State::Playing);
}

/// A track silent past the timeout is excluded, so video alone resumes.
#[test]
fn silent_track_is_excluded_after_the_timeout() {
    let mut s = started_sim();
    let last_audio = s.now;
    while s.pauses().is_empty() {
        s.step(false, true);
        assert!(s.now - last_audio < 2_900_000, "never starved");
    }
    assert!(s.resumes().is_empty());
    let timeout = cfg().track_silence_timeout_us;
    while s.resumes().is_empty() {
        s.step(false, true);
        assert!(
            s.now - last_audio < 2 * timeout + 1_000_000,
            "never excluded"
        );
    }
    let resumed_after = s.resumes()[0] - last_audio;
    assert!(
        resumed_after >= timeout,
        "resumed after only {resumed_after} us of audio silence"
    );
    assert!(resumed_after < timeout + 2 * STEP_US);
    // Audio coming back is part of the minimum again.
    assert_eq!(s.c.stats().limiting_track, Some(VIDEO));
    s.c.on_track_media(s.now, AUDIO, at(BASE, s.t_us));
    assert_eq!(s.c.stats().limiting_track, Some(AUDIO));
}

/// A registered track that never delivers blocks the first fill, then is
/// excluded after the timeout.
#[test]
fn registered_but_silent_track_blocks_then_times_out() {
    let mut s = Sim::new(cfg());
    s.c.on_tick(s.now, None); // the arrival clock is read before the registration
    let registered_at = s.now;
    s.c.register_track(AUDIO);
    s.c.on_track_media(s.now, VIDEO, BASE);
    while s.starts().is_empty() {
        s.now += STEP_US;
        s.t_us += STEP_US;
        s.c.on_track_media(s.now, VIDEO, at(BASE, s.t_us + 2_500_000));
        s.tick();
        assert!(s.now - registered_at < 10_000_000, "never started");
    }
    let waited = s.starts()[0].0 - registered_at;
    let timeout = cfg().track_silence_timeout_us;
    assert!(
        waited >= timeout,
        "started after {waited} us, before the timeout"
    );
    assert!(waited < timeout + 2 * STEP_US);
}

/// A global delivery gap (nothing delivered anywhere) excludes nobody: the
/// effective end stays the MIN of the last ends, not the latest.
#[test]
fn global_gap_keeps_the_minimum_of_the_last_ends() {
    let mut s = Sim::new(cfg());
    s.c.on_track_media(s.now, AUDIO, BASE);
    s.c.on_track_media(s.now, VIDEO, BASE);
    s.now += STEP_US;
    s.c.on_track_media(s.now, AUDIO, at(BASE, 1_000_000));
    s.c.on_track_media(s.now, VIDEO, at(BASE, 4_000_000));
    s.tick();
    // min = audio at 1.0 s: below the 1.8 s start threshold.
    assert!(s.starts().is_empty());
    assert_eq!(s.c.stats().limiting_track, Some(AUDIO));
    let gap_end = s.now + 2 * cfg().track_silence_timeout_us;
    while s.now < gap_end {
        s.now += STEP_US;
        s.tick();
    }
    assert!(s.starts().is_empty(), "started on stale video end in a gap");
    assert_eq!(s.c.stats().limiting_track, Some(AUDIO));
    // Audio returns: the start lead is the audio (minimum) end.
    s.c.on_track_media(s.now, AUDIO, at(BASE, 3_000_000));
    s.tick();
    let starts = s.starts();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].1, 3_000_000);
}

/// The integrator's repro: Pause with audio starving, then a global gap longer
/// than the silence timeout. No Resume during the gap; Resume only once audio
/// data is back and its lead satisfied; no second stall.
#[test]
fn global_gap_after_pause_does_not_resume_on_stale_data() {
    let mut s = started_sim();
    let stopped = s.now;
    while s.pauses().is_empty() {
        s.step(false, true);
        assert!(s.now - stopped < 2_900_000, "never starved");
    }
    assert_eq!(s.c.state(), State::Starved);
    // Global gap: nothing delivered for well over the timeout.
    let gap_end = s.now + cfg().track_silence_timeout_us + 2_000_000;
    while s.now < gap_end {
        s.now += STEP_US;
        s.tick();
        assert!(s.resumes().is_empty(), "Resume during a global gap");
    }
    assert_eq!(s.c.state(), State::Starved);
    assert_eq!(s.c.stats().limiting_track, Some(AUDIO));
    // Data returns on both tracks; Resume only after audio is delivered.
    let back = s.now;
    while s.resumes().is_empty() {
        s.step(true, true);
        assert!(s.now - back < 6_000_000, "never resumed");
    }
    assert!(s.resumes()[0] > back);
    assert_eq!(s.pauses().len(), 1, "second stall after the gap");
    assert_eq!(s.c.state(), State::Playing);
}

/// A registered track that never delivered is not excluded by a global gap:
/// it blocks until another track delivers again (only then is it silent
/// relative to a delivering track).
#[test]
fn never_delivered_track_is_excluded_only_while_others_deliver() {
    let mut s = Sim::new(cfg());
    s.c.on_tick(s.now, None);
    s.c.register_track(AUDIO);
    // One big video delivery (enough lead to start), then nothing at all.
    s.c.on_track_media(s.now, VIDEO, BASE);
    s.now += STEP_US;
    s.c.on_track_media(s.now, VIDEO, at(BASE, 2_500_000));
    let end = s.now + 3 * cfg().track_silence_timeout_us;
    while s.now < end {
        s.now += STEP_US;
        s.tick();
        assert!(s.starts().is_empty(), "started in a global gap");
    }
    // Video delivers again: AUDIO is now silent relative to it and excluded.
    s.c.on_track_media(s.now, VIDEO, at(BASE, 2_600_000));
    s.tick();
    assert_eq!(s.starts().len(), 1);
}

/// `on_media` is track-0 shorthand: it joins the per-track minimum.
#[test]
fn on_media_is_track_zero_shorthand_and_mixes_with_tracks() {
    let mut a = Sim::new(cfg());
    let mut b = Sim::new(cfg());
    a.c.on_media(a.now, BASE);
    b.c.on_track_media(b.now, DEFAULT_TRACK, BASE);
    a.c.on_track_media(a.now, VIDEO, BASE);
    b.c.on_track_media(b.now, VIDEO, BASE);
    for _ in 0..200 {
        for s in [&mut a, &mut b] {
            s.now += STEP_US;
            s.t_us += STEP_US;
        }
        a.c.on_media(a.now, at(BASE, a.t_us));
        b.c.on_track_media(b.now, DEFAULT_TRACK, at(BASE, b.t_us));
        a.c.on_track_media(a.now, VIDEO, at(BASE, a.t_us + VIDEO_AHEAD_US));
        b.c.on_track_media(b.now, VIDEO, at(BASE, b.t_us + VIDEO_AHEAD_US));
        a.tick();
        b.tick();
    }
    assert!(!a.starts().is_empty());
    assert_eq!(a.log, b.log);
    // Track 0 (the `on_media` side) is the minimum and was the one waited on.
    assert_eq!(a.c.stats().limiting_track, Some(DEFAULT_TRACK));
}

#[test]
fn single_track_never_names_a_track() {
    let mut s = Sim::new(cfg());
    s.c.on_media(s.now, BASE);
    while s.starts().is_empty() {
        s.now += STEP_US;
        s.t_us += STEP_US;
        s.c.on_media(s.now, at(BASE, s.t_us));
        s.tick();
    }
    assert_eq!(s.c.stats().limiting_track, None);
    // Run the renderer past the data: a starvation pause names no track.
    while s.pauses().is_empty() {
        s.now += STEP_US;
        s.tick();
        assert!(s.now < 60_000_000);
    }
    assert_eq!(s.pauses()[0].1, None);
    let rec = s
        .log
        .iter()
        .find(|(_, e)| matches!(e, Event::Pause { .. }))
        .unwrap()
        .1
        .to_record();
    assert_eq!((rec.track, rec.track_valid), (0, 0));
}

#[test]
fn rebaseline_forgets_ends_but_keeps_registrations() {
    let mut s = started_sim();
    s.c.register_track(7);
    s.c.on_hint(s.now, buffering_runtime::Hint::SyncLoss);
    s.drain();
    assert_eq!(s.c.state(), State::Buffering);
    let before = s.log.len();
    // Plenty of audio and video, but registered track 7 never delivers: the
    // refill is blocked (a forgotten registration would resume at once).
    let t0 = s.now;
    while s.now - t0 < 2_500_000 {
        s.t_us += STEP_US;
        s.now += STEP_US;
        s.c.on_track_media(s.now, AUDIO, at(BASE, s.t_us + 5_000_000));
        s.c.on_track_media(s.now, VIDEO, at(BASE, s.t_us + 5_000_000));
        s.tick();
    }
    assert_eq!(
        s.log.len(),
        before,
        "refill started despite registered silent track"
    );
    assert_eq!(s.c.state(), State::Buffering);
}

#[test]
fn track_cap_is_enforced_without_panicking() {
    let mut c = BufferController::new(cfg(), None).unwrap();
    for t in 0..(MAX_TRACKS as u32 + 8) {
        c.on_track_media(1_000_000, t, BASE);
        c.register_track(t + 100);
    }
    c.on_tick(1_100_000, None);
    let _ = c.stats();
}

#[test]
fn silence_timeout_is_validated() {
    assert_eq!(cfg().track_silence_timeout_us, 3_000_000);
    let bad = cfg().with_track_silence_timeout_us(0);
    assert_eq!(
        bad,
        Err(buffering_runtime::Error::InvalidConfig(
            buffering_runtime::ConfigField::TrackSilence
        ))
    );
    assert_eq!(
        cfg()
            .with_track_silence_timeout_us(500_000)
            .unwrap()
            .track_silence_timeout_us,
        500_000
    );
}

/// ABI v2 layout: 40 bytes, `track`/`track_valid` appended after `b_us`.
#[test]
fn event_record_layout_is_abi_v2() {
    assert_eq!(buffering_runtime::ABI_VERSION, 2);
    assert_eq!(core::mem::size_of::<EventRecord>(), 40);
    assert_eq!(core::mem::align_of::<EventRecord>(), 8);
    let r = EventRecord::default();
    let base = core::ptr::addr_of!(r) as usize;
    let off = |p: usize| p - base;
    assert_eq!(off(core::ptr::addr_of!(r.kind) as usize), 0);
    assert_eq!(off(core::ptr::addr_of!(r.reason) as usize), 4);
    assert_eq!(off(core::ptr::addr_of!(r.media_27mhz) as usize), 8);
    assert_eq!(off(core::ptr::addr_of!(r.a_us) as usize), 16);
    assert_eq!(off(core::ptr::addr_of!(r.b_us) as usize), 24);
    assert_eq!(off(core::ptr::addr_of!(r.track) as usize), 32);
    assert_eq!(off(core::ptr::addr_of!(r.track_valid) as usize), 36);
}

/// Removing a replaced track un-gates the first fill: a registered track that
/// never delivers (the old audio) blocks until `unregister_track`.
#[test]
fn unregister_replaced_track_ungates_start() {
    let mut s = Sim::new(cfg());
    s.c.on_tick(s.now, None);
    s.c.register_track(AUDIO);
    s.c.on_track_media(s.now, VIDEO, BASE);
    for _ in 0..30 {
        s.now += STEP_US;
        s.t_us += STEP_US;
        s.c.on_track_media(s.now, VIDEO, at(BASE, s.t_us + 2_500_000));
        s.tick();
    }
    assert!(s.starts().is_empty(), "registered silent track must block");
    s.c.unregister_track(AUDIO);
    s.tick();
    assert_eq!(s.starts().len(), 1, "unregister must release the start");
}

/// Same for a resume: audio replaced while paused no longer gates it, and an
/// unknown track is a no-op.
#[test]
fn unregister_replaced_track_ungates_resume() {
    let mut s = started_sim();
    s.c.unregister_track(99); // unknown: no-op
    assert_eq!(s.c.stats().limiting_track, Some(AUDIO));
    // Audio stops (replaced); video keeps going, so the lead is audio-limited
    // and starves.
    while s.pauses().is_empty() {
        s.step(false, true);
        assert!(s.t_us < 30_000_000, "never paused");
    }
    assert_eq!(s.pauses()[0].1, Some(AUDIO));
    s.c.unregister_track(AUDIO);
    assert_eq!(s.c.stats().limiting_track, None, "one track left");
    s.step(false, true);
    s.step(false, true);
    assert_eq!(s.resumes().len(), 1, "video alone must now resume");
}

/// Removing the last track leaves no effective end; ticks are inert and a
/// later delivery works again.
#[test]
fn unregister_last_track_is_inert() {
    let mut s = Sim::new(cfg());
    s.c.on_track_media(s.now, AUDIO, BASE);
    s.c.unregister_track(AUDIO);
    s.tick();
    assert!(s.starts().is_empty());
    assert_eq!(s.c.stats().limiting_track, None);
    s.c.on_track_media(s.now, VIDEO, BASE);
    while s.starts().is_empty() {
        s.now += STEP_US;
        s.t_us += STEP_US;
        s.c.on_track_media(s.now, VIDEO, at(BASE, s.t_us + 2_500_000));
        s.tick();
        assert!(s.t_us < 10_000_000, "never started");
    }
    assert_eq!(s.starts().len(), 1);
}

/// A `PcrDiscontinuity` while playing yields `Pause` then `Resume`, never a
/// second `StartPlaying`: callers must treat both as "go".
#[test]
fn discontinuity_while_playing_resumes_not_restarts() {
    let mut s = started_sim();
    s.c.on_hint(s.now, buffering_runtime::Hint::PcrDiscontinuity);
    s.drain();
    while s.resumes().is_empty() {
        s.step(true, true);
        assert!(s.t_us < 30_000_000, "never resumed");
    }
    assert_eq!(s.starts().len(), 1, "no second StartPlaying");
    assert_eq!(s.pauses().len(), 1);
    assert_eq!(s.resumes().len(), 1);
}

/// Caller contract (join point): the first reported end is the start origin.
/// Audio reported BEFORE the join point (audio is muxed ~1 s behind video in
/// TS) anchors the origin early, so start fires with ~1 s less audio cushion
/// past the join point than when the first report is from the join point.
#[test]
fn audio_before_the_join_point_anchors_start_early() {
    const EARLY_US: u64 = 1_000_000;
    // Both runs deliver identical ends from the join point (BASE) on; they
    // differ only in the first audio report.
    let run = |audio_first: Ticks27| {
        let mut s = Sim::new(cfg());
        s.c.on_track_media(s.now, AUDIO, audio_first);
        s.c.on_track_media(s.now, VIDEO, BASE);
        while s.starts().is_empty() {
            s.step(true, true);
            assert!(s.t_us < 10_000_000, "never started");
        }
        s.t_us // audio cushion past the join point at start
    };
    let early = run(BASE - EARLY_US * TICKS_PER_US);
    let from_join = run(BASE);
    assert!(
        from_join >= cfg().policy.initial_us,
        "join-point-first start with only {from_join} us of audio"
    );
    assert!(
        early + EARLY_US <= from_join + 2 * STEP_US && early + 2 * STEP_US < from_join,
        "early origin should lose ~{EARLY_US} us: early {early}, from join {from_join}"
    );
}
