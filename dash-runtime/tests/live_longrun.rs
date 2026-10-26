//! A simulated day of live playback: the client's bookkeeping must stay
//! bounded however long the session runs (DESIGN 8, "long-run bounded
//! memory"), with and without Period churn.

mod common;

use common::checks::*;
use common::scenario::*;
use common::synth::Addressing;
use common::world::{MICROS, Sim};
use dash_runtime::client::{DashConfig, StateSize};

const HOUR_US: u64 = 3_600 * MICROS as u64;
const REMEMBERED_DONE: usize = 4096;

fn sample(sim: &Sim) -> StateSize {
    sim.client.state_size()
}

fn bounded(s: &StateSize) {
    assert!(s.periods <= 4, "{s:?}");
    assert!(s.requests <= 24, "{s:?}");
    assert!(s.inflight <= 12, "{s:?}");
    assert!(s.parked <= 8, "{s:?}");
    assert!(s.failures <= 8, "{s:?}");
    assert!(s.done <= REMEMBERED_DONE, "{s:?}");
    assert!(s.inits <= 8, "{s:?}");
    assert!(s.reps <= 16, "{s:?}");
    assert!(s.queued <= 16, "{s:?}");
    assert!(s.outputs <= 64, "{s:?}");
    assert!(s.live <= 8, "{s:?}");
}

fn day(mut sim: Sim, label: &str) -> Vec<StateSize> {
    sim.keep_samples = false;
    let mut sizes = Vec::new();
    for _ in 0..24 {
        sim.run_for(HOUR_US);
        assert!(sim.fatal().is_none(), "{label}: {:?}", sim.fatal());
        let s = sample(&sim);
        bounded(&s);
        sizes.push(s);
    }
    assert!(sim.max_stall_run < 3, "{label}");
    assert!(
        sim.samples_per_track().values().all(|n| *n > 100_000),
        "{label}"
    );
    sizes
}

fn steady(sizes: &[StateSize]) {
    let first = sizes[3];
    let last = sizes[23];
    assert_eq!(first.periods, last.periods);
    assert_eq!(first.inits, last.inits);
    assert_eq!(first.reps, last.reps);
    assert!(last.requests <= first.requests + 12);
}

#[test]
fn twenty_four_hours_of_a_single_period_keeps_every_collection_bounded() {
    let mut synth = with_utc(Addressing::TimelineNumber);
    synth.tsb_us = Some(60 * MICROS);
    let (sim, geo) = sim_with(synth, 41 * MICROS + 120_000, DashConfig::default());
    let sizes = day(sim, "single period");
    steady(&sizes);
    let _ = geo;
}

#[test]
fn twenty_four_hours_with_a_new_period_every_ten_minutes_keeps_every_collection_bounded() {
    let mut synth = with_utc(Addressing::TimelineNumber);
    synth.tsb_us = Some(60 * MICROS);
    synth.period_starts_us = (0..160).map(|i| i * 600 * MICROS).collect();
    synth.announce_ahead_us = 8 * MICROS;
    let (mut sim, geo) = sim_with(synth, 41 * MICROS + 120_000, DashConfig::default());
    sim.keep_samples = false;
    let mut sizes = Vec::new();
    for _ in 0..24 {
        sim.run_for(HOUR_US);
        assert!(sim.fatal().is_none(), "{:?}", sim.fatal());
        let s = sample(&sim);
        bounded(&s);
        sizes.push(s);
    }
    steady(&sizes);
    let reqs = segment_requests(&sim, geo);
    assert_no_duplicates(&reqs);
    let changes = sim.count(|o| matches!(o, dash_runtime::client::Output::PeriodChanged { .. }));
    assert!(changes >= 140, "{changes} Period changes in a day");
}
