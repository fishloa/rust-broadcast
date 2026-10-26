//! The live client against the `World` simulation: availability decided from
//! TRUE time, a host clock with skew, a synthesised origin recycling real
//! fMP4 segments (see `common/world.rs`).

mod common;

use common::checks::*;
use common::scenario::*;
use common::synth::{Addressing, MPD_URL, Synth, UTC_HOST};
use common::world::{MICROS, Sim, World};
use dash_runtime::client::{DashConfig, Output, PullLimits};
use dash_runtime::timing::clock::ClockSource;

#[test]
fn host_clock_number_template_joins_near_the_edge_and_plays_without_gaps() {
    let synth = Synth::new(AST_US, Addressing::Number);
    let (mut sim, geo) = sim_with(synth, 40 * MICROS + 300_000, DashConfig::default());
    sim.give_host_clock();
    let start_utc = sim.world.true_utc();
    sim.run_for(40 * MICROS as u64);
    no_fatal(&sim);
    assert!(sim.errors.is_empty(), "{:?}", sim.errors);
    let reqs = segment_requests(&sim, geo);
    assert!(!reqs.is_empty());
    assert_contiguous(&reqs);
    assert_no_duplicates(&reqs);
    let sources = clock_sources(&sim);
    assert_eq!(sources.first().map(|s| s.0), Some(ClockSource::Host));
    let first = first_index(&reqs, "V").unwrap();
    let edge = geo.complete_count(start_utc) - 1;
    let delay_segs = 3;
    assert!(
        (first - (edge - delay_segs)).abs() <= 1,
        "join {first} edge {edge}"
    );
    let per_track = sim.samples_per_track();
    assert_eq!(per_track.len(), 2);
    assert!(per_track.values().all(|n| *n > 0));
    let _ = UTC_HOST;
    assert!(sim.count(|o| matches!(o, Output::Samples { .. })) > 0);
}

#[test]
fn every_addressing_joins_at_the_delay_and_plays_on_utc_timing() {
    for a in [
        Addressing::Number,
        Addressing::TimelineNumber,
        Addressing::TimelineTime,
    ] {
        let ast = if a == Addressing::Number { AST_US } else { 0 };
        let mut synth = Synth::new(ast, a).with_utc("http-xsdate", UTC_XSDATE);
        synth.start_number = if a == Addressing::TimelineNumber {
            895_967_208
        } else {
            1
        };
        let geo = synth.geo();
        let world = World::new(AST_US + 41 * MICROS + 120_000);
        let mut sim = Sim::new(
            MPD_URL,
            DashConfig::default(),
            PullLimits::default(),
            world,
            Box::new(synth),
        );
        let start_utc = sim.world.true_utc();
        let reqs = run_checked(&mut sim, geo, 30);
        assert!(early_requests(&reqs, geo, TOLERANCE_US).is_empty(), "{a:?}");
        let first = first_index(&reqs, "V").unwrap();
        let edge = geo.complete_count(start_utc) - 1;
        assert!(
            (first - (edge - 3)).abs() <= 1,
            "{a:?}: join {first} edge {edge}"
        );
        let sources = clock_sources(&sim);
        assert_eq!(
            sources.first().map(|s| s.0),
            Some(ClockSource::UtcTiming),
            "{a:?}"
        );
        let requested: Vec<_> = sim.requests(common::world::Kind::Utc);
        assert!(!requested.is_empty());
    }
}

#[test]
fn host_skew_with_utc_timing_never_requests_early() {
    for skew in [
        -60 * MICROS,
        -5 * MICROS,
        -500_000,
        500_000,
        5 * MICROS,
        60 * MICROS,
    ] {
        let (mut sim, geo) = sim_with(
            with_utc(Addressing::Number),
            41 * MICROS + 120_000,
            DashConfig::default(),
        );
        sim.world.host_skew_us = skew;
        sim.give_host_clock();
        let reqs = run_checked(&mut sim, geo, 25);
        assert!(
            early_requests(&reqs, geo, TOLERANCE_US).is_empty(),
            "skew {skew}"
        );
        assert_eq!(
            clock_sources(&sim).first().map(|s| s.0),
            Some(ClockSource::UtcTiming)
        );
    }
}

#[test]
fn host_skew_without_utc_timing_is_absorbed_as_early_requests() {
    for skew in [500_000i64, 5 * MICROS] {
        let mut synth = Synth::new(AST_US, Addressing::Number);
        synth.early_status = 425;
        let (mut sim, geo) = sim_with(synth, 41 * MICROS + 120_000, DashConfig::default());
        sim.world.host_skew_us = skew;
        sim.give_host_clock();
        sim.run_for(90 * MICROS as u64);
        no_fatal(&sim);
        let reqs = segment_requests(&sim, geo);
        assert_contiguous(&reqs);
        assert_no_duplicates(&reqs);
        let early = early_requests(&reqs, geo, TOLERANCE_US);
        assert!(!early.is_empty(), "skew {skew}");
        let counted = sim
            .classes
            .iter()
            .filter(|c| matches!(c, dash_runtime::client::FailureClass::Retry { .. }))
            .count();
        assert_eq!(counted, 0, "early answers are never counted");
        assert!(sim.classes.iter().any(|c| matches!(
            c,
            dash_runtime::client::FailureClass::NotYetAvailable { .. }
        )));
        assert!(sim.count(|o| matches!(o, Output::Samples { .. })) > 0);
    }
}

#[test]
fn a_host_clock_a_minute_ahead_waits_then_catches_up_with_discontinuities() {
    let mut synth = Synth::new(AST_US, Addressing::Number);
    synth.early_status = 425;
    let (mut sim, geo) = sim_with(synth, 41 * MICROS + 120_000, DashConfig::default());
    sim.world.host_skew_us = 60 * MICROS;
    sim.give_host_clock();
    sim.run_for(120 * MICROS as u64);
    no_fatal(&sim);
    let reqs = segment_requests(&sim, geo);
    assert_no_duplicates(&reqs);
    assert!(sim.count(|o| matches!(o, Output::Discontinuity)) >= 1);
    assert!(sim.count(|o| matches!(o, Output::Samples { .. })) > 0);
    let per_second = reqs.len() as f64 / 120.0;
    assert!(per_second <= 8.0, "request rate {per_second}/s");
}

#[test]
fn date_only_clock_is_a_usable_fallback() {
    let mut synth = Synth::new(AST_US, Addressing::Number);
    synth.send_date = true;
    let (mut sim, geo) = sim_with(synth, 41 * MICROS + 120_000, DashConfig::default());
    let reqs = run_checked(&mut sim, geo, 30);
    let sources = clock_sources(&sim);
    assert_eq!(sources.first().map(|s| s.0), Some(ClockSource::MpdDate));
    assert!(early_requests(&reqs, geo, 600_000).is_empty());
    let _ = UTC_HOST;
}
