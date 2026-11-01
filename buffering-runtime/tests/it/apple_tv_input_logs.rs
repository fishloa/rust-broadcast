//! Apple TV controller INPUT-LOG goldens. The zenith-tv player recorded, for
//! three clean-ish runs, every call it made into the controller in call order
//! (`private/fixtures/buffering-runtime/*-apple-tv.lb.csv`, see its PROVENANCE.md):
//! `kind,now_us,a,b` with `chunk` (a = bytes, b = PCR 27 MHz or empty),
//! `media` (a = track 1 video / 2 audio, b = media end 27 MHz), `tick`
//! (a = has_pos, b = playback 27 MHz), `register` (a = track), `hint`
//! (a = code). The logs are replayed verbatim through the CURRENT controller
//! (`PolicyConfig::DEFAULT` with `initial_us` 500 ms, no seed), so unlike the
//! per-chunk traces no renderer model is needed: the logged ticks ARE the
//! renderer. The controller's decisions are therefore not what the device saw
//! (the playback positions follow the old controller's decisions); the goldens
//! pin what the current controller emits for the recorded inputs.
//!
//! Loaded through `test_fixtures::require` (skips loudly without the private
//! repo; fails under `REQUIRE_PRIVATE_FIXTURES=1`).

use buffering_runtime::{
    BufferController, ControllerConfig, Event, Hint, Micros, PolicyConfig, Reason, Ticks27,
};

const VIDEO: u32 = 1;
const AUDIO: u32 = 2;
/// Hint codes as logged by zenith-tv (none occur in the committed logs; an
/// unknown code is a test failure rather than a silent skip).
const HINT_CODES: &[Hint] = &[
    Hint::InputDegraded,
    Hint::InputRecovered,
    Hint::SyncLoss,
    Hint::PcrDiscontinuity,
    Hint::SourceEnded,
    Hint::SourceError,
    Hint::Reconnected,
];

struct Replay {
    events: Vec<(Micros, Event)>,
    final_target: Micros,
    stalls: u32,
    /// Every `(now, target)` seen via `TargetChanged`.
    targets: Vec<(Micros, Micros)>,
    chunks: usize,
}

impl Replay {
    fn pauses(&self) -> Vec<(Micros, &Event)> {
        self.events
            .iter()
            .filter(|(_, e)| matches!(e, Event::Pause { .. }))
            .map(|(t, e)| (*t, e))
            .collect()
    }
    fn starts(&self) -> Vec<Micros> {
        self.events
            .iter()
            .filter(|(_, e)| matches!(e, Event::StartPlaying { .. }))
            .map(|(t, _)| *t)
            .collect()
    }
    /// Target in force at `now` (the `min_us` floor before any change: the
    /// controller starts at the floor, `initial_us` only sets the start fill).
    fn target_at(&self, now: Micros) -> Micros {
        self.targets
            .iter()
            .take_while(|(t, _)| *t <= now)
            .last()
            .map_or(FLOOR_US, |(_, n)| *n)
    }
}

const INITIAL_US: Micros = 500_000;
const FLOOR_US: Micros = 300_000;

fn config() -> ControllerConfig {
    config_with(|c| c)
}

fn config_with(f: impl FnOnce(ControllerConfig) -> ControllerConfig) -> ControllerConfig {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::DEFAULT;
    p.initial_us = INITIAL_US;
    f(ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us).unwrap())
}

fn replay(name: &str, cfg: ControllerConfig) -> Option<Replay> {
    replay_with(name, cfg, &[], false)
}

/// Closed-loop renderer: plays at 1x from where the controller says to start or
/// resume and holds where it says to pause, replacing the logged playback
/// positions (which follow the OLD controller's decisions).
struct Renderer {
    clock: Ticks27,
    since: Micros,
    running: bool,
}

impl Renderer {
    fn pos(&self, now: Micros) -> Ticks27 {
        if self.running {
            self.clock + (now - self.since) * 27
        } else {
            self.clock
        }
    }
}

/// `closed`: drive ticks from a [`Renderer`] instead of the logged positions.
fn replay_with(name: &str, cfg: ControllerConfig, pre: &[u32], closed: bool) -> Option<Replay> {
    let mut rend = Renderer {
        clock: 0,
        since: 0,
        running: false,
    };
    let path = test_fixtures::require(
        &format!("buffering-runtime/{name}-apple-tv.lb.csv"),
        "buffering-runtime apple_tv_input_logs",
    )?;
    let text = std::fs::read_to_string(&path).unwrap();
    let mut c = BufferController::new(cfg, None).unwrap();
    let mut r = Replay {
        events: Vec::new(),
        final_target: 0,
        stalls: 0,
        targets: Vec::new(),
        chunks: 0,
    };
    let mut t0 = None::<u64>;
    for line in text.lines().skip(1) {
        let mut f = line.split(',');
        let kind = f.next().unwrap();
        let now_raw: u64 = f.next().unwrap().parse().unwrap();
        let a = f.next().unwrap();
        let b = f.next().unwrap_or("");
        let first_row = t0.is_none();
        let t0 = *t0.get_or_insert(now_raw);
        let now = now_raw - t0;
        if first_row {
            for t in pre {
                c.register_track(*t);
            }
        }
        match kind {
            "chunk" => {
                r.chunks += 1;
                let pcr: Option<Ticks27> = (!b.is_empty()).then(|| b.parse().unwrap());
                c.on_chunk(now, a.parse().unwrap(), pcr);
            }
            "media" => {
                let track: u32 = a.parse().unwrap();
                assert!(track == VIDEO || track == AUDIO, "unknown track {track}");
                c.on_track_media(now, track, b.parse().unwrap());
            }
            "tick" => {
                let pos = if closed {
                    Some(rend.pos(now))
                } else {
                    (a == "1").then(|| b.parse::<Ticks27>().unwrap())
                };
                c.on_tick(now, pos);
            }
            "register" => c.register_track(a.parse().unwrap()),
            "hint" => c.on_hint(now, HINT_CODES[a.parse::<usize>().unwrap()]),
            other => panic!("unknown row kind {other}"),
        }
        while let Some(e) = c.poll_event() {
            match e {
                Event::TargetChanged { new_us, .. } => r.targets.push((now, new_us)),
                Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                    rend = Renderer {
                        clock: media_27mhz,
                        since: now,
                        running: true,
                    };
                }
                Event::Pause { hold_at_27mhz, .. } => {
                    rend = Renderer {
                        clock: hold_at_27mhz,
                        since: now,
                        running: false,
                    };
                }
                _ => {}
            }
            r.events.push((now, e));
        }
    }
    let s = c.stats();
    r.final_target = s.target_us;
    r.stalls = s.stalls;
    Some(r)
}

/// Largest gap between consecutive audio `media` rows at or after `from_us`,
/// measured straight from the log (independent of the controller).
fn max_audio_gap_us(name: &str, from_us: u64) -> u64 {
    let path = test_fixtures::require(
        &format!("buffering-runtime/{name}-apple-tv.lb.csv"),
        "buffering-runtime apple_tv_input_logs",
    )
    .unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let mut t0 = None::<u64>;
    let (mut last, mut max) = (None::<u64>, 0);
    for line in text.lines().skip(1) {
        let mut f = line.split(',');
        let kind = f.next().unwrap();
        let now: u64 = f.next().unwrap().parse().unwrap();
        let now = now - *t0.get_or_insert(now);
        if kind == "media" && f.next() == Some("2") {
            if let Some(l) = last
                && now >= from_us
            {
                max = max.max(now - l);
            }
            last = Some(now);
        }
    }
    max
}

fn pause_summary(r: &Replay) -> Vec<(Micros, Option<u32>, Reason, Micros, Micros)> {
    r.pauses()
        .into_iter()
        .map(|(t, e)| match e {
            Event::Pause {
                reason,
                depth_us,
                resume_target_us,
                track,
                ..
            } => (t, *track, *reason, *depth_us, *resume_target_us),
            _ => unreachable!(),
        })
        .collect()
}

const MARGIN_US: Micros = 100_000;

/// rai-3 (clean 60 s Apple TV run): MP2 audio arrives in ~0.3-0.4 s bursts.
/// Golden = what the CURRENT controller emits for the recorded inputs.
#[test]
fn rai3_clean_golden() {
    let Some(r) = replay("rai-3", config()) else {
        return;
    };
    assert_eq!(r.chunks, 11_108);
    assert_eq!(r.starts(), [1_337_896]);
    assert!(r.pauses().is_empty(), "clean run must not pause");
    assert_eq!(r.stalls, 0);
    let targets: Vec<(Micros, Micros)> = r.targets.clone();
    assert_eq!(
        targets,
        [
            (1_034_306, 412_000),
            (2_000_467, 560_000),
            (41_599_553, 547_968),
            (41_754_551, 532_470),
            (41_880_402, 519_889),
            (42_004_074, 507_524),
            (42_104_964, 497_438),
            (42_215_220, 486_415),
            (42_320_108, 475_929),
            (42_442_949, 463_647),
            (42_556_018, 452_342),
            (42_685_313, 439_414),
            (42_789_532, 428_994),
            (42_896_204, 418_329),
        ]
    );
    // the steady ~0.3 s audio cadence keeps refreshing the granularity floor
    // (step 312 ms + 100 ms margin), so the target no longer decays to 360 ms
    assert_eq!(r.final_target, 412_000);
    // The audio really is delivered in bursts of at least 0.3 s (from the log
    // itself), and once seen the target covers it plus the margin.
    let gap = max_audio_gap_us("rai-3", 0);
    assert!(gap >= 300_000, "log audio gap {gap}");
    let t = r.target_at(5_000_000);
    assert!(
        t >= 300_000 + MARGIN_US,
        "target {t} below granularity+margin"
    );
    assert_eq!(t, 560_000);
}

/// The golden above is not vacuous, and the two target drivers are told
/// apart. The log's media rows are per demuxed MP2 frame (24 ms steps, a PES
/// of ~0.3 s per chunk); the controller coalesces them per delivery, so the
/// granularity floor engages (before the 2.0 s PDV lift: 412 ms = 312 ms step
/// plus margin at 1.03 s). With the PDV term at its minimum the floor alone holds
/// 412 ms; with the floor off too the target stays at the 300 ms floor. (Until
/// the coalescing the floor never engaged on this log.)
#[test]
fn rai3_target_lift_comes_from_pdv_and_the_granularity_floor() {
    let pdv_off = |gran: bool| {
        config_with(|c| {
            let mut p = c.policy;
            p.k1_q8 = 1;
            ControllerConfig::new(c.jitter, p, c.starve_threshold_us, c.nodata_after_us)
                .unwrap()
                .with_track_granularity(gran)
        })
    };
    // Skip loudly when the private fixture is unavailable (the publish lane
    // does not check the private repo out), like every other test here.
    if replay("rai-3", config()).is_none() {
        return;
    }
    let floor_only = replay("rai-3", pdv_off(true)).unwrap();
    assert_eq!(floor_only.target_at(5_000_000), 412_000);
    assert_eq!(floor_only.targets.first(), Some(&(1_034_306, 412_000)));
    let neither = replay("rai-3", pdv_off(false)).unwrap();
    assert_eq!(neither.target_at(5_000_000), FLOOR_US);
    assert!(neither.targets.is_empty());
    // floor off, PDV on: the 560 ms PDV lift only, no early 412 ms step
    let on = replay("rai-3", config()).unwrap();
    let off = replay("rai-3", config_with(|c| c.with_track_granularity(false))).unwrap();
    assert_eq!(off.targets.first(), Some(&(2_000_467, 560_000)));
    assert_eq!(on.target_at(5_000_000), off.target_at(5_000_000));
    assert_ne!(on.targets, off.targets);
}

/// france-5 (clean): audio gaps ~0.2 s, below the 300 ms floor, so the target
/// never moves and nothing pauses.
#[test]
fn france5_clean_golden() {
    let Some(r) = replay("france-5", config()) else {
        return;
    };
    assert_eq!(r.chunks, 19_909);
    assert_eq!(r.starts(), [1_257_133]);
    assert!(r.pauses().is_empty());
    assert_eq!(r.stalls, 0);
    assert!(r.targets.is_empty(), "targets {:?}", r.targets);
    assert_eq!(r.final_target, 300_000);
    let gap = max_audio_gap_us("france-5", 5_000_000);
    assert!(gap < 300_000, "log audio gap {gap}");
}

/// The stalling rai-3 session (old controller f512d3eaa paused 4x, all audio:
/// leads -63 / 12 / 15 / 9 ms). Replayed through the CURRENT controller the
/// log produces 3 audio `DataArriving` pauses (2 at the former 1.5 x
/// `after_rebuffer`: the lower 1.25 x resume bars (375/515 ms vs 450/618 ms) run
/// into the device's logged positions once more at 2.073 s, an open-loop artefact
/// the closed-loop assertions below do not show; 3 before the per-delivery
/// coalescing made the floor engage on these per-frame media rows): the first
/// ~97 ms after `StartPlaying` (audio is registered only AFTER the start, see
/// `rai3_stall_pauses_explained_late_registration_and_open_loop`), then the floor lifts the target to
/// 412 ms (step 312 ms + margin) at 849 ms; the second pause at 1.417 s
/// happens although the floor is in force (lead 16 ms: the open-loop replay's
/// logged positions follow the OLD controller's earlier starts, so this is not
/// a prediction for the device). The old 4th pause no longer occurs. NOTE: this is
/// an open-loop replay (the logged ticks carry the old controller's playback
/// positions), so the golden pins the controller's decisions for these
/// inputs, not what the device would have seen.
#[test]
fn rai3_stall_golden_current_behaviour() {
    let Some(r) = replay("rai-3-stall", config()) else {
        return;
    };
    assert_eq!(r.starts(), [528_980]);
    assert_eq!(
        pause_summary(&r),
        [
            (625_716, Some(AUDIO), Reason::DataArriving, 0, 375_000),
            (
                1_417_065,
                Some(AUDIO),
                Reason::DataArriving,
                15_860,
                515_000
            ),
            (2_072_729, Some(AUDIO), Reason::DataArriving, 9_582, 617_597),
        ]
    );
    assert_eq!(r.stalls, 3);
    assert_eq!(
        r.targets,
        [
            (625_716, 359_765),
            (848_712, 412_000),
            (1_417_065, 494_078),
            (2_072_729, 592_507)
        ]
    );
    assert_eq!(r.final_target, 592_507);
    // after the last pause the run is clean: no pause past 3 s
    assert!(r.pauses().iter().all(|(t, _)| *t < 3_000_000));
}

/// rai-3 recorded with PER-CHUNK media reporting (zenith 9e2d027: one audio end
/// per delivered PES, 195 audio calls in 60 s, step median 0.288 s / max
/// 0.312 s; the device saw 0 pauses, first frame 2.65 s). Replayed through the
/// current controller: no pause, start at 1.362 s (the per-frame rai-3 log
/// starts at 1.338 s: same shape, same start within 25 ms), and the
/// granularity floor lifts the target to 412 ms (312 ms max step + margin) at
/// 1.076 s, before the start; the 600/680 ms lifts at ~45 s are PDV. The floor
/// is proven to be the cause with the PDV term at its minimum: floor on holds
/// 412 ms from 1.076 s, floor off stays at 300 ms (mutation: disabling the
/// floor).
#[test]
fn rai3_perchunk_golden() {
    let Some(r) = replay("rai-3-perchunk", config()) else {
        return;
    };
    assert_eq!(r.chunks, 10_967);
    assert_eq!(r.starts(), [1_361_559]);
    assert!(r.pauses().is_empty());
    assert_eq!(r.stalls, 0);
    assert_eq!(
        r.targets,
        [
            (1_075_848, 412_000),
            (45_527_825, 600_000),
            (46_030_568, 680_000)
        ]
    );
    assert_eq!(r.final_target, 680_000);
    // the floor is in force before the start
    assert!(r.target_at(1_361_559) >= 300_000 + MARGIN_US);

    let pdv_off = |gran: bool| {
        config_with(|c| {
            let mut p = c.policy;
            p.k1_q8 = 1;
            ControllerConfig::new(c.jitter, p, c.starve_threshold_us, c.nodata_after_us)
                .unwrap()
                .with_track_granularity(gran)
        })
    };
    let floor_only = replay("rai-3-perchunk", pdv_off(true)).unwrap();
    assert_eq!(floor_only.targets.first(), Some(&(1_075_848, 412_000)));
    assert_eq!(floor_only.target_at(5_000_000), 412_000);
    let neither = replay("rai-3-perchunk", pdv_off(false)).unwrap();
    // (a late PDV event at ~45 s may still move it; nothing before 40 s)
    assert!(neither.targets.iter().all(|(t, _)| *t > 40_000_000));
    assert_eq!(neither.target_at(5_000_000), FLOOR_US);
}

/// The two stall-log pauses, explained (analysis of the log, 2026-10-17).
///
/// * **Pause 1 (0.626 s, depth 0) is a late track registration, not an unknown
///   granularity.** The log registers video at 0.179 s but audio only at
///   0.622 s, 93 ms AFTER `StartPlaying` (0.529 s); the first audio end
///   (43417.017) is 73 ms behind the playback position by then. The controller
///   cannot wait for a track nobody announced. Registering both tracks at the
///   first row (what a caller that knows its PMT does) makes the start wait for
///   audio's first delivery (1.148 s) and pause 1 disappears.
/// * **Pause 2 (1.417 s, lead 16 ms) is an open-loop artefact.** The logged
///   playback position is the DEVICE's: the old controller resumed it at ~0.88 s
///   and it ran 1x to 43417.601, while the controller under test (after its own
///   pause 1) resumes at 1.148 s from 43417.017 and would be at ~43417.29, a
///   lead of ~0.33 s. Driven by a closed-loop 1x renderer instead of the logged
///   positions, the same recorded inputs give no pause at all.
#[test]
fn rai3_stall_pauses_explained_late_registration_and_open_loop() {
    let Some(r) = replay_with("rai-3-stall", config(), &[VIDEO, AUDIO], false) else {
        return;
    };
    assert_eq!(r.starts(), [1_147_776]);
    let p = pause_summary(&r);
    // two open-loop pauses remain (1.25 x resume bar: the lower resume leads run
    // into the device's logged positions again at 2.073 s; was one at 1.5 x)
    assert_eq!(p.len(), 2, "{p:?}");
    assert_eq!(p[0].0, 1_417_065, "first open-loop pause");
    assert_eq!(p[1].0, 2_072_729, "second open-loop pause (1.25 x bar)");
    // closed loop, tracks registered up front: no stall
    let c = replay_with("rai-3-stall", config(), &[VIDEO, AUDIO], true).unwrap();
    assert_eq!(c.starts(), [1_147_776]);
    assert!(c.pauses().is_empty(), "{:?}", pause_summary(&c));
    assert_eq!(c.stalls, 0);
    // closed loop with the log's own late registration: the unannounced audio
    // costs exactly the first pause (it joins behind the position the
    // video-only start already played); the second pause is gone
    let late = replay_with("rai-3-stall", config(), &[], true).unwrap();
    assert_eq!(late.starts(), [528_980]);
    assert_eq!(
        pause_summary(&late),
        [(625_716, Some(AUDIO), Reason::DataArriving, 0, 375_000)]
    );
    assert_eq!(late.stalls, 1);
}

/// The start gate (`LEARN_STEPS` closed delivery windows per delivering track)
/// on the real Apple TV input logs, closed loop, tracks registered up front.
/// Measured against the same replay with the gate bypassed: at `initial_us`
/// 500 ms and 1.8 s the gate costs NOTHING (the start is bound by the audio
/// lead filling in lumps of one PES, 0.3 s: lead 703 ms at the rai-3 start for
/// a 500 ms fill), and with a single learnt step instead of two the
/// 300 ms fill starts rai-3 / rai-3-perchunk at 1.034 s / 1.076 s and STALLS
/// on both. So the two-step gate stays and these starts are pinned: raising
/// `LEARN_STEPS` moves them later, lowering it stalls.
#[test]
fn start_gate_is_free_at_500ms_and_prevents_the_300ms_stall() {
    // (log, start at 300 ms fill, start at 500 ms fill, start at 1.8 s fill)
    const CASES: [(&str, Micros, Micros, Micros); 4] = [
        ("rai-3", 1_231_966, 1_337_896, 2_559_618),
        ("rai-3-perchunk", 1_217_953, 1_361_559, 2_553_123),
        ("rai-3-stall", 1_147_776, 1_147_776, 2_347_173),
        ("france-5", 1_174_068, 1_257_133, 2_622_974),
    ];
    for (name, s300, s500, s1800) in CASES {
        for (initial_us, want) in [(300_000, s300), (500_000, s500), (1_800_000, s1800)] {
            let cfg = config_with(|mut c| {
                c.policy.initial_us = initial_us;
                c
            });
            let Some(r) = replay_with(name, cfg, &[VIDEO, AUDIO], true) else {
                return;
            };
            assert_eq!(r.starts(), [want], "{name} initial {initial_us}");
            assert!(
                r.pauses().is_empty(),
                "{name} initial {initial_us}: {:?}",
                pause_summary(&r)
            );
            assert_eq!(r.stalls, 0, "{name} initial {initial_us}");
        }
    }
}
