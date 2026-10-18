//! Sparse still-image slate start (zenith-tv Apple TV pipeline log) and the
//! rai-gulp 600 s clean baseline, replayed verbatim through the current
//! controller (`PolicyConfig::DEFAULT`, `initial_us` 500 ms, no seed; format
//! `kind,now_us,a,b` as in `apple_tv_input_logs.rs`; fixtures in
//! `private/fixtures/buffering-runtime/`, see PROVENANCE.md).
//!
//! The slate log: a programme with audio + video, then a PCR discontinuity
//! (hint 4) and a video-only still-image slate, one I-picture every 2.29 s
//! (each reported as a zero-length join then end = pts + 2.29 s). Audio was
//! registered at the start and never delivers on the new timeline. Measured
//! from the hint, with that registered-but-never-delivering audio track the
//! `Resume` used to come at 6.876 s (the third slate delivery: the silence
//! clock only ran at deliveries, so the 3 s timeout was first reached by the
//! delivery 4.59 s after the first); it now comes at 5.30 s (3 s after the
//! first delivery: the timeout in wall terms). A caller that unregisters the
//! audio track at the discontinuity (it knows the new PMT) resumes at the
//! first delivery, 2.288 s. The 0.7b start gate is NOT the cause: after a
//! rebaseline a retained track keeps its closed-window count, so the gate is
//! already open.

use buffering_runtime::{
    BufferController, ControllerConfig, Event, Hint, Micros, PolicyConfig, Ticks27,
};

const AUDIO: u32 = 2;

struct Replay {
    /// `(time since the first row, event)`.
    events: Vec<(Micros, Event)>,
    hint_at: Option<Micros>,
    final_target: Micros,
    stalls: u32,
}

impl Replay {
    fn pauses(&self) -> usize {
        self.events
            .iter()
            .filter(|(_, e)| matches!(e, Event::Pause { .. }))
            .count()
    }
    fn starts(&self) -> Vec<Micros> {
        self.events
            .iter()
            .filter(|(_, e)| matches!(e, Event::StartPlaying { .. }))
            .map(|(t, _)| *t)
            .collect()
    }
    /// `Resume` instants after the hint, relative to it.
    fn resumes_after_hint(&self) -> Vec<Micros> {
        let h = self.hint_at.unwrap();
        self.events
            .iter()
            .filter(|(t, e)| *t >= h && matches!(e, Event::Resume { .. }))
            .map(|(t, _)| *t - h)
            .collect()
    }
}

fn config(f: impl FnOnce(ControllerConfig) -> ControllerConfig) -> ControllerConfig {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::DEFAULT;
    p.initial_us = 500_000;
    f(ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us).unwrap())
}

fn replay(name: &str, cfg: ControllerConfig, unregister_audio_at_hint: bool) -> Option<Replay> {
    let path = test_fixtures::require(
        &format!("buffering-runtime/{name}.lb.csv"),
        "buffering-runtime slate_start",
    )?;
    let text = std::fs::read_to_string(path).unwrap();
    let mut c = BufferController::new(cfg, None).unwrap();
    let mut r = Replay {
        events: Vec::new(),
        hint_at: None,
        final_target: 0,
        stalls: 0,
    };
    let mut t0 = None::<u64>;
    for line in text.lines().filter(|l| !l.starts_with("kind,")) {
        let mut f = line.split(',');
        let kind = f.next().unwrap();
        let raw: u64 = f.next().unwrap().parse().unwrap();
        let a = f.next().unwrap();
        let b = f.next().unwrap_or("");
        let now = raw - *t0.get_or_insert(raw);
        match kind {
            "chunk" => c.on_chunk(
                now,
                a.parse().unwrap(),
                (!b.is_empty()).then(|| b.parse::<Ticks27>().unwrap()),
            ),
            "media" => c.on_track_media(now, a.parse().unwrap(), b.parse().unwrap()),
            "tick" => c.on_tick(now, (a == "1").then(|| b.parse::<Ticks27>().unwrap())),
            "register" => c.register_track(a.parse().unwrap()),
            "unregister" => c.unregister_track(a.parse().unwrap()),
            "hint" => {
                assert_eq!(a, "4", "only PcrDiscontinuity occurs in these logs");
                r.hint_at = Some(now);
                c.on_hint(now, Hint::PcrDiscontinuity);
                if unregister_audio_at_hint {
                    c.unregister_track(AUDIO);
                }
            }
            other => panic!("unknown row kind {other}"),
        }
        while let Some(e) = c.poll_event() {
            r.events.push((now, e));
        }
    }
    let s = c.stats();
    r.final_target = s.target_us;
    r.stalls = s.stalls;
    Some(r)
}

const SLATE: &str = "stimmungsgarten-slate-apple-tv-pipeline";
const GULP: &str = "rai-gulp-apple-tv-baseline";

/// Golden for the slate log.
#[test]
fn slate_resume_golden() {
    let Some(r) = replay(SLATE, config(|c| c), false) else {
        return;
    };
    assert_eq!(r.starts().len(), 1);
    assert_eq!(r.resumes_after_hint(), [5_303_186]);
}

/// rai-gulp (clean 600 s Apple TV baseline): one StartPlaying, 0 pauses.
#[test]
fn rai_gulp_baseline_golden() {
    let Some(r) = replay(GULP, config(|c| c), false) else {
        return;
    };
    assert_eq!(r.starts(), [596_990]);
    assert_eq!(r.pauses(), 0);
    assert_eq!(r.stalls, 0);
    assert_eq!(r.final_target, 412_000);
}

/// The caller-side recipe: unregistering the audio track at the discontinuity
/// (the new PMT carries no audio) releases the slate at its first delivery.
#[test]
fn slate_unregistered_audio_resumes_at_first_delivery() {
    let Some(r) = replay(SLATE, config(|c| c), true) else {
        return;
    };
    assert_eq!(r.resumes_after_hint(), [2_288_364]);
}

/// The slate shape without a fixture. Programme: video every 100 ms (0.9 s
/// ahead) and audio every 300 ms, both registered; then a PCR discontinuity
/// and a video-only sparse-but-regular source (one picture per 2.29 s, a
/// zero-length join then end = pts + step). The registered audio never
/// delivers on the new timeline: the resume waits the silence timeout in wall
/// terms (3 s after the first slate delivery), not for the delivery at which
/// the delivery-driven silence clock first reaches it (the third, 4.58 s).
#[test]
fn sparse_slate_with_registered_silent_audio_resumes_at_the_silence_timeout() {
    const STEP: Micros = 2_290_000;
    const TICK: Micros = 10_000;
    const MS: u64 = 27_000;
    let mut c = BufferController::new(config(|c| c), None).unwrap();
    c.register_track(1);
    c.register_track(AUDIO);
    let mut played = false;
    // programme: 6 s, video + audio, a closed-loop 1x renderer after the start
    let mut now = 0;
    let mut pos: Ticks27 = 0;
    let mut clock = (0, 0);
    while now < 6_000_000 {
        if now % 100_000 == 0 {
            c.on_chunk(now, 2_000, Some(now * 27));
            let v = (now + 900_000) * 27;
            if now == 0 {
                c.on_track_media(now, 1, 0);
                c.on_track_media(now, AUDIO, 0);
            }
            c.on_track_media(now, 1, v);
            if now % 300_000 == 0 {
                c.on_track_media(now, AUDIO, now * 27 + 300 * MS);
            }
        }
        if played {
            pos = clock.0 + (now - clock.1) * 27;
        }
        c.on_tick(now, played.then_some(pos));
        while let Some(e) = c.poll_event() {
            if let Event::StartPlaying { media_27mhz, .. } = e {
                played = true;
                clock = (media_27mhz, now);
            }
        }
        now += TICK;
    }
    assert!(played, "the programme starts");
    let hint = now;
    c.on_hint(hint, Hint::PcrDiscontinuity);
    while c.poll_event().is_some() {}
    let first = hint + 1_000_000;
    let mut end: Ticks27 = 90_000_000_000;
    let mut resume = None;
    let mut next = first;
    let mut now = hint;
    while now < first + 10_000_000 && resume.is_none() {
        if now >= first && (now - first) % 100_000 == 0 {
            // the slate picture arrives as ~50 KB of chunks every 100 ms of
            // its 2.29 s cadence slot, the PCR clock running in real time
            c.on_chunk(now, 2_300, Some(90_000_000_000 + (now - first) * 27));
        }
        if now >= next {
            if next == first {
                c.on_track_media(now, 1, end);
            }
            end += STEP * 27;
            c.on_track_media(now, 1, end);
            next += STEP;
        }
        c.on_tick(now, None);
        while let Some(e) = c.poll_event() {
            if matches!(e, Event::Resume { .. }) {
                resume = Some(now - first);
            }
        }
        now += TICK;
    }
    let s = resume.expect("the slate resumes");
    assert!(
        (3_000_000..3_100_000).contains(&s),
        "resume {s} us after the first slate delivery"
    );
}
