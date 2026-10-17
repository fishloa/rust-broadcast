//! Real-trace golden tests: the eight zenith-tv delivery traces (private
//! fixtures repo, `private/fixtures/live-buffer/`, see its PROVENANCE.md)
//! replayed through the controller, asserted against an INDEPENDENT oracle.
//!
//! Oracle: `docs/live-buffer/analysis/golden_oracle.py` (a second, integer,
//! from-the-design implementation in Python, same tick schedule: 5 ms virtual
//! ticks plus one at every chunk arrival). Regenerate the numbers below with
//!
//! ```text
//! python3 docs/live-buffer/analysis/golden_oracle.py                  # initial 1800 ms
//! python3 docs/live-buffer/analysis/golden_oracle.py --initial-ms 500
//! ```
//!
//! The older sample-driven `controller_sim.py` model agrees on the headline
//! decisions (zero-stall start, zap 1814.662 / 1824.370 / 3124.917 ms for
//! tf-1 / 6ter / servustv at initial 1800 ms).
//!
//! The traces carry PCR only; the playback (PTS) domain is derived as PCR
//! minus a constant offset. The tests prove the domain separation: decisions
//! are invariant under a constant PCR-PTS offset (positions shift by exactly
//! it, across the 27 MHz wrap too), and a mis-mapped renderer clock shifts the
//! starvation instant by exactly the error.
//!
//! These tests load through `test_fixtures::require`: they skip loudly when
//! the private repo is not checked out and fail when `REQUIRE_PRIVATE_FIXTURES=1`
//! (CI) and it is missing.

mod support;

use live_buffer::Event;
use support::*;

fn trace(name: &str) -> Option<Vec<Chunk>> {
    test_fixtures::require(
        &format!("live-buffer/{name}.csv"),
        "live-buffer real_traces",
    )
    .map(|p| load_trace(&p))
}

struct Golden {
    name: &'static str,
    start_us: u64,
    /// (pause_us, resume_us) pairs.
    pauses: &'static [(u64, u64)],
    rebaselined: usize,
    min_lead_us: i64,
    target_min: u64,
    target_max: u64,
    target_final: u64,
    target_sum: u64,
}

/// `python3 docs/live-buffer/analysis/golden_oracle.py` (initial 1800 ms).
const GOLDEN_1800: &[Golden] = &[
    Golden {
        name: "t18",
        start_us: 1_817_414,
        pauses: &[],
        rebaselined: 0,
        min_lead_us: 1_759_135,
        target_min: 300_000,
        target_max: 300_000,
        target_final: 300_000,
        target_sum: 516_900_000,
    },
    Golden {
        name: "rai-1",
        start_us: 1_816_004,
        pauses: &[],
        rebaselined: 0,
        min_lead_us: 1_780_453,
        target_min: 300_000,
        target_max: 300_000,
        target_final: 300_000,
        target_sum: 724_500_000,
    },
    Golden {
        name: "rai-sport",
        start_us: 1_798_640,
        pauses: &[],
        rebaselined: 0,
        min_lead_us: 1_757_045,
        target_min: 300_000,
        target_max: 300_000,
        target_final: 300_000,
        target_sum: 722_700_000,
    },
    Golden {
        name: "orf1",
        start_us: 1_828_196,
        pauses: &[],
        rebaselined: 0,
        min_lead_us: 1_771_650,
        target_min: 300_000,
        target_max: 300_000,
        target_final: 300_000,
        target_sum: 620_400_000,
    },
    Golden {
        name: "orf-sport",
        start_us: 1_819_355,
        pauses: &[],
        rebaselined: 0,
        min_lead_us: 1_772_713,
        target_min: 300_000,
        target_max: 300_000,
        target_final: 300_000,
        target_sum: 630_900_000,
    },
    // servustv: startup gap raises the start target (zap 3.12 s), then the
    // reconnect at 17.26 s pauses (Discontinuity) and resumes at the target.
    Golden {
        name: "servustv",
        start_us: 3_124_917,
        pauses: &[(17_264_170, 18_916_952)],
        rebaselined: 1,
        min_lead_us: 1_568_793,
        target_min: 300_000,
        target_max: 1_602_372,
        target_final: 1_602_372,
        target_sum: 2_491_381_344,
    },
    // 6ter / tf-1: the 1.42 / 1.34 s gap is SURVIVED at initial 1.8 s; the
    // lowest lead left is 349.9 / 383.4 ms (5 ms-tick resolution).
    Golden {
        name: "6ter",
        start_us: 1_824_370,
        pauses: &[],
        rebaselined: 0,
        min_lead_us: 349_920,
        target_min: 300_000,
        target_max: 1_735_273,
        target_final: 1_735_273,
        target_sum: 1_807_415_887,
    },
    Golden {
        name: "tf-1",
        start_us: 1_814_662,
        pauses: &[],
        rebaselined: 0,
        min_lead_us: 383_420,
        target_min: 300_000,
        target_max: 1_680_980,
        target_final: 1_680_980,
        target_sum: 1_010_757_700,
    },
];

fn check(g: &Golden, r: &Run) {
    let n = g.name;
    assert_eq!(r.start(), Some(g.start_us), "{n}: start");
    let p: Vec<(u64, u64)> = r.pauses().iter().map(|&(a, b)| (a, b.unwrap())).collect();
    assert_eq!(p, g.pauses, "{n}: pauses");
    assert_eq!(
        r.count(|e| matches!(e, Event::Rebaselined)),
        g.rebaselined,
        "{n}: rebaselined"
    );
    assert_eq!(r.min_lead_playing, Some(g.min_lead_us), "{n}: min lead");
    assert_eq!(
        *r.targets.iter().min().unwrap(),
        g.target_min,
        "{n}: target min"
    );
    assert_eq!(
        *r.targets.iter().max().unwrap(),
        g.target_max,
        "{n}: target max"
    );
    assert_eq!(
        *r.targets.last().unwrap(),
        g.target_final,
        "{n}: target final"
    );
    assert_eq!(
        r.targets.iter().sum::<u64>(),
        g.target_sum,
        "{n}: target sum"
    );
}

#[test]
fn golden_decisions_initial_1800_match_the_python_oracle() {
    for g in GOLDEN_1800 {
        let Some(chunks) = trace(g.name) else { return };
        check(g, &replay(&chunks, &Opts::new(1_800_000)));
    }
}

#[test]
fn gap_free_traces_never_pause_and_target_stays_at_the_floor() {
    for name in ["t18", "rai-1", "rai-sport", "orf1", "orf-sport"] {
        let Some(chunks) = trace(name) else { return };
        for initial in [500_000, 1_800_000] {
            let r = replay(&chunks, &Opts::new(initial));
            assert!(r.pauses().is_empty(), "{name}@{initial}: false pause");
            assert!(
                r.targets.iter().all(|&t| t == 300_000),
                "{name}: target left the floor"
            );
            assert_eq!(r.ctl.stats().stalls, 0);
        }
    }
}

#[test]
fn the_default_initial_target_survives_the_tf1_and_6ter_gaps() {
    for (name, lead_lo, lead_hi) in [("tf-1", 340_000, 400_000), ("6ter", 330_000, 400_000)] {
        let Some(chunks) = trace(name) else { return };
        let r = replay(&chunks, &Opts::new(1_800_000));
        assert_eq!(r.ctl.stats().stalls, 0, "{name} stalled");
        let lead = r.min_lead_playing.unwrap();
        assert!(
            (lead_lo..=lead_hi).contains(&lead),
            "{name}: lead left {lead} us outside the survivability band"
        );
    }
}

/// `python3 docs/live-buffer/analysis/golden_oracle.py --initial-ms 500`:
/// the old 500 ms start stalls once at the gap and resumes ~1.75 s later.
#[test]
fn golden_initial_500_stalls_once_at_the_gap() {
    for (name, start, pause, resume, sum) in [
        ("6ter", 525_799, 25_135_000, 27_856_113, 1_807_415_887),
        ("tf-1", 500_679, 46_370_000, 49_016_033, 1_010_757_700),
    ] {
        let Some(chunks) = trace(name) else { return };
        let r = replay(&chunks, &Opts::new(500_000));
        assert_eq!(r.start(), Some(start), "{name}");
        assert_eq!(r.pauses(), vec![(pause, Some(resume))], "{name}");
        assert_eq!(r.targets.iter().sum::<u64>(), sum, "{name}");
        // reason NoData: nothing arrived for the whole gap
        let reason = r.events.iter().find_map(|(_, e)| match e {
            Event::Pause { reason, .. } => Some(*reason),
            _ => None,
        });
        assert_eq!(reason, Some(live_buffer::Reason::NoData));
    }
}

fn positions(r: &Run) -> Vec<(u64, &'static str, u64)> {
    r.events
        .iter()
        .filter_map(|(t, e)| match *e {
            Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                Some((*t, e.name(), media_27mhz))
            }
            Event::Pause { hold_at_27mhz, .. } => Some((*t, e.name(), hold_at_27mhz)),
            _ => None,
        })
        .collect()
}

#[test]
fn decisions_are_invariant_under_a_constant_pcr_pts_offset() {
    for name in ["6ter", "servustv"] {
        let Some(chunks) = trace(name) else { return };
        let base_opts = Opts::new(500_000);
        let base = replay(&chunks, &base_opts);
        let base_pos = positions(&base);
        assert!(!base_pos.is_empty());
        // offsets incl. one that puts the PTS domain across the 27 MHz wrap
        for off_ms in [0i64, 250, 1_000, -700, 1_000_000_000] {
            let mut o = base_opts.clone();
            o.offset_ticks = off_ms * 1000 * TICKS_PER_US as i64;
            let r = replay(&chunks, &o);
            let pos = positions(&r);
            assert_eq!(pos.len(), base_pos.len(), "{name} off {off_ms}");
            let delta = base_opts.offset_ticks - o.offset_ticks;
            for (a, b) in base_pos.iter().zip(&pos) {
                assert_eq!(
                    (a.0, a.1),
                    (b.0, b.1),
                    "{name} off {off_ms}: timeline changed"
                );
                assert_eq!(
                    b.2,
                    shift27(a.2, delta),
                    "{name} off {off_ms}: position not shifted by exactly the offset"
                );
            }
            assert_eq!(
                r.targets, base.targets,
                "{name}: target series depends on the offset"
            );
        }
    }
}

#[test]
fn a_mismapped_renderer_clock_shifts_the_starvation_instant_by_exactly_the_error() {
    let Some(chunks) = trace("tf-1") else { return };
    let base = replay(&chunks, &Opts::new(500_000));
    let (t0, _) = base.pauses()[0];
    for err_ms in [100u64, 250] {
        let mut o = Opts::new(500_000);
        o.clock_err_ticks = (err_ms * 1000 * TICKS_PER_US) as i64;
        let r = replay(&chunks, &o);
        let (t, _) = r.pauses()[0];
        let shift = t0 as i64 - t as i64;
        let err = (err_ms * 1000) as i64;
        assert!(
            (shift - err).abs() <= o.tick_us as i64,
            "reported clock +{err_ms} ms moved the pause by {shift} us, expected {err} +- one tick"
        );
    }
}
