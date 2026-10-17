//! Apple TV controller INPUT-LOG goldens. The zenith-tv player recorded, for
//! three clean-ish runs, every call it made into the controller in call order
//! (`private/fixtures/live-buffer/*-apple-tv.lb.csv`, see its PROVENANCE.md):
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

use live_buffer::{
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
    let path = test_fixtures::require(
        &format!("live-buffer/{name}-apple-tv.lb.csv"),
        "live-buffer apple_tv_input_logs",
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
        let t0 = *t0.get_or_insert(now_raw);
        let now = now_raw - t0;
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
                let pos = (a == "1").then(|| b.parse::<Ticks27>().unwrap());
                c.on_tick(now, pos);
            }
            "register" => c.register_track(a.parse().unwrap()),
            "hint" => c.on_hint(now, HINT_CODES[a.parse::<usize>().unwrap()]),
            other => panic!("unknown row kind {other}"),
        }
        while let Some(e) = c.poll_event() {
            if let Event::TargetChanged { new_us, .. } = e {
                r.targets.push((now, new_us));
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
        &format!("live-buffer/{name}-apple-tv.lb.csv"),
        "live-buffer apple_tv_input_logs",
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
/// log produces 2 audio `DataArriving` pauses (3 before the per-delivery
/// coalescing made the floor engage on these per-frame media rows): the first
/// ~97 ms after `StartPlaying`, before any granularity has been learnt (the
/// start-up gap, flagged for follow-up), then the floor lifts the target to
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
            (625_716, Some(AUDIO), Reason::DataArriving, 0, 450_000),
            (
                1_417_065,
                Some(AUDIO),
                Reason::DataArriving,
                15_860,
                618_000
            ),
        ]
    );
    assert_eq!(r.stalls, 2);
    assert_eq!(
        r.targets,
        [(625_716, 359_765), (848_712, 412_000), (1_417_065, 494_078)]
    );
    assert_eq!(r.final_target, 494_078);
    // after the last pause the run is clean: no pause past 3 s
    assert!(r.pauses().iter().all(|(t, _)| *t < 3_000_000));
}
