//! The clock ladder and `UTCTiming` sampling through the `World` simulation
//! (DESIGN 3.3, 3.4 and the case list of 3.6).

mod common;

use common::checks::*;
use common::scenario::*;
use common::synth::{Addressing, UtcMode};
use common::world::{Kind, MICROS, Method};
use dash_runtime::client::{DashClockPolicy, DashConfig, FailureClass, FatalKind, Output};
use dash_runtime::timing::clock::ClockSource;

const START: i64 = 41 * MICROS + 120_000;

fn quick() -> DashConfig {
    DashConfig::default()
}

fn first_source(sim: &common::world::Sim) -> Option<ClockSource> {
    clock_sources(sim).first().map(|s| s.0)
}

#[test]
fn a_utc_server_ahead_of_the_origin_makes_early_requests_that_are_absorbed() {
    let mut synth = with_utc(Addressing::Number);
    synth.early_status = 425;
    synth.utc.skew_us = 3 * MICROS;
    let (mut sim, geo) = sim_with(synth, START, quick());
    sim.run_for(60 * MICROS as u64);
    no_fatal(&sim);
    let reqs = segment_requests(&sim, geo);
    assert_contiguous(&reqs);
    assert_no_duplicates(&reqs);
    assert!(!early_requests(&reqs, geo, TOLERANCE_US).is_empty());
    assert_eq!(first_source(&sim), Some(ClockSource::UtcTiming));
    assert!(
        sim.classes
            .iter()
            .all(|c| !matches!(c, FailureClass::Retry { .. }))
    );
}

#[test]
fn a_utc_server_behind_the_origin_joins_late_but_plays_cleanly() {
    let mut synth = with_utc(Addressing::Number);
    synth.utc.skew_us = -4 * MICROS;
    let (mut sim, geo) = sim_with(synth, START, quick());
    let reqs = run_checked(&mut sim, geo, 40);
    assert!(early_requests(&reqs, geo, TOLERANCE_US).is_empty());
    let first = first_index(&reqs, "V").unwrap();
    let edge = geo.complete_count(AST_US + START) - 1;
    assert!((first - (edge - 5)).abs() <= 1, "join {first} edge {edge}");
}

#[test]
fn direct_utc_timing_anchors_at_the_mpd_exchange_midpoint() {
    let mut synth = common::synth::Synth::new(AST_US, Addressing::Number);
    synth.utc_direct = true;
    let (mut sim, geo) = sim_with(synth, START, quick());
    let reqs = run_checked(&mut sim, geo, 30);
    assert_eq!(first_source(&sim), Some(ClockSource::UtcTiming));
    assert!(
        sim.requests(Kind::Utc).is_empty(),
        "direct needs no request"
    );
    assert!(early_requests(&reqs, geo, 25_000).is_empty());
}

#[test]
fn prefer_host_uses_the_host_clock_and_never_asks_a_utc_server() {
    let mut dash = quick();
    dash.clock_policy = DashClockPolicy::PreferHost;
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, dash);
    sim.give_host_clock();
    run_checked(&mut sim, geo, 30);
    assert_eq!(first_source(&sim), Some(ClockSource::Host));
    assert!(sim.requests(Kind::Utc).is_empty());
}

#[test]
fn prefer_host_without_a_host_clock_falls_back_to_utc_timing() {
    let mut dash = quick();
    dash.clock_policy = DashClockPolicy::PreferHost;
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, dash);
    run_checked(&mut sim, geo, 30);
    assert_eq!(first_source(&sim), Some(ClockSource::UtcTiming));
}

#[test]
fn head_refused_with_405_is_reissued_as_get_and_still_anchors() {
    let mut synth = common::synth::Synth::new(AST_US, Addressing::Number)
        .with_utc("http-head", "http://utc.test/head");
    synth.utc.head_status = Some(405);
    let (mut sim, geo) = sim_with(synth, START, quick());
    run_checked(&mut sim, geo, 20);
    let utc = sim.requests(Kind::Utc);
    assert_eq!(utc[0].method, Method::Head);
    assert_eq!(utc[0].status, Some(405));
    assert_eq!(utc[1].method, Method::Get);
    assert_eq!(first_source(&sim), Some(ClockSource::UtcTiming));
}

#[test]
fn garbage_empty_and_huge_utc_bodies_fall_through_to_the_host_clock() {
    for mode in [UtcMode::Garbage, UtcMode::Empty, UtcMode::Huge] {
        let mut synth = with_utc(Addressing::Number);
        synth.utc.mode = mode;
        let (mut sim, geo) = sim_with(synth, START, quick());
        sim.give_host_clock();
        run_checked(&mut sim, geo, 20);
        assert_eq!(first_source(&sim), Some(ClockSource::Host), "{mode:?}");
    }
}

#[test]
fn a_utc_timeout_falls_to_the_next_element_then_to_the_host_clock() {
    let mut synth = common::synth::Synth::new(AST_US, Addressing::Number)
        .with_utc("http-xsdate", "http://utc.test/first")
        .with_utc("http-xsdate", "http://utc.test/second");
    synth.utc.by_path.insert("first".into(), UtcMode::Hang);
    let (mut sim, geo) = sim_with(synth, START, quick());
    run_checked(&mut sim, geo, 20);
    assert_eq!(first_source(&sim), Some(ClockSource::UtcTiming));
    let utc = sim.requests(Kind::Utc);
    assert!(utc[0].url.ends_with("/first"));
    assert!(utc[1].url.ends_with("/second"));
    let gap = utc[1].send_mono_us - utc[0].send_mono_us;
    assert_eq!(gap, quick().utc_timeout_us);

    let mut synth = with_utc(Addressing::Number);
    synth.utc.mode = UtcMode::Hang;
    let (mut sim, geo) = sim_with(synth, START, quick());
    sim.give_host_clock();
    run_checked(&mut sim, geo, 30);
    assert_eq!(first_source(&sim), Some(ClockSource::Host));
    let first_segment = sim.requests(Kind::Segment)[0];
    let first_mpd = sim.requests(Kind::Mpd)[0];
    assert!(first_segment.send_mono_us - first_mpd.send_mono_us >= quick().utc_timeout_us);
}

#[test]
fn a_utc_round_trip_over_the_limit_is_discarded_and_retried_once() {
    let mut dash = quick();
    dash.utc_max_rtt_us = MICROS as u64;
    dash.utc_timeout_us = 4 * MICROS as u64;
    dash.utc_wait_us = 12 * MICROS as u64;
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, dash);
    sim.rtt_utc_us = Some(2 * MICROS as u64);
    sim.give_host_clock();
    run_checked(&mut sim, geo, 30);
    assert_eq!(sim.requests(Kind::Utc).len(), 2, "one retry, then give up");
    assert_eq!(first_source(&sim), Some(ClockSource::Host));
}

#[test]
fn a_utc_round_trip_inside_the_limit_is_used_with_a_wider_error_bar() {
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, quick());
    sim.rtt_utc_us = Some(1_600_000);
    run_checked(&mut sim, geo, 30);
    let s = clock_sources(&sim);
    assert_eq!(s[0].0, ClockSource::UtcTiming);
    assert!(s[0].1 >= 800_000, "{:?}", s);
}

#[test]
fn no_clock_source_at_all_is_a_typed_fatal_error() {
    let synth = common::synth::Synth::new(AST_US, Addressing::Number);
    let (mut sim, _) = sim_with(synth, START, quick());
    sim.run_for(5 * MICROS as u64);
    assert_eq!(sim.fatal(), Some(&FatalKind::ClockUnavailable));
    assert!(sim.requests(Kind::Segment).is_empty());
    assert!(sim.requests(Kind::Init).len() <= 2);
}

#[test]
fn a_static_mpd_needs_no_clock_and_no_date() {
    let mut synth = common::synth::Synth::new(AST_US, Addressing::Number);
    synth.static_from_utc_us = Some(AST_US);
    let (mut sim, _) = sim_with(synth, 12 * MICROS, quick());
    sim.run_for(60 * MICROS as u64);
    assert!(sim.client.presentation_ended(), "{:?}", sim.fatal());
    assert!(sim.requests(Kind::Utc).is_empty());
    assert!(clock_sources(&sim).is_empty());
    assert!(sim.count(|o| matches!(o, Output::EndOfStream)) == 1);
}

#[test]
fn the_host_clock_stepping_forward_and_back_never_duplicates_a_segment() {
    for step in [30 * MICROS, -30 * MICROS] {
        let synth = common::synth::Synth::new(AST_US, Addressing::Number);
        let (mut sim, geo) = sim_with(synth, START, quick());
        sim.give_host_clock();
        sim.run_for(30 * MICROS as u64);
        sim.world.host_step_us = step;
        sim.give_host_clock();
        sim.run_for(150 * MICROS as u64);
        no_fatal(&sim);
        let reqs = segment_requests(&sim, geo);
        assert_no_duplicates(&reqs);
        assert!(sim.max_stall_run < 3);
        assert!(
            sim.count(|o| matches!(o, Output::Samples { .. })) > 20,
            "step {step}"
        );
        let per_second = reqs.len() as f64 / 180.0;
        assert!(per_second <= 8.0, "rate {per_second}");
    }
}

#[test]
fn six_hours_at_plus_and_minus_100_ppm_with_resync_never_requests_early() {
    for ppm in [100i64, -100] {
        let mut dash = quick();
        dash.utc_resync_us = 600 * MICROS as u64;
        let mut synth = with_utc(Addressing::Number);
        synth.tsb_us = Some(60 * MICROS);
        let (mut sim, geo) = sim_with(synth, START, dash);
        sim.world.host_drift_ppm = ppm;
        sim.run_for(6 * 3600 * MICROS as u64);
        no_fatal(&sim);
        assert!(sim.errors.is_empty(), "{:?}", sim.errors);
        let reqs = segment_requests(&sim, geo);
        assert_no_duplicates(&reqs);
        assert_contiguous(&reqs);
        assert!(early_requests(&reqs, geo, 150_000).is_empty(), "ppm {ppm}");
        let utc = sim.requests(Kind::Utc).len();
        assert!((35..=200).contains(&utc), "{utc} UTC requests");
    }
}

#[test]
fn early_answers_trigger_a_rate_limited_clock_resync_even_without_periodic_resync() {
    let mut dash = quick();
    dash.utc_resync_us = u64::MAX;
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, dash);
    sim.world.host_drift_ppm = 100;
    sim.run_for(3 * 3600 * MICROS as u64);
    let reqs = segment_requests(&sim, geo);
    assert!(early_requests(&reqs, geo, 150_000).is_empty());
    let utc = sim.requests(Kind::Utc);
    assert!(utc.len() > 3, "{} UTC requests", utc.len());
    for w in utc.windows(2) {
        assert!(
            w[1].send_mono_us - w[0].send_mono_us >= 30 * MICROS as u64 - 1_000,
            "forced resyncs closer than 30 s"
        );
    }
}

#[test]
fn a_utc_server_clock_step_at_a_resync_is_adopted_without_gap_or_duplicate() {
    let mut dash = quick();
    dash.utc_resync_us = 20 * MICROS as u64;
    let (mut sim, geo, synth) = sim_shared(with_utc(Addressing::Number), START, dash);
    sim.run_for(30 * MICROS as u64);
    synth.borrow_mut().utc.skew_us = 30 * MICROS;
    sim.run_for(200 * MICROS as u64);
    no_fatal(&sim);
    let reqs = segment_requests(&sim, geo);
    assert_no_duplicates(&reqs);
    assert!(sim.max_stall_run < 3);
    assert!(sim.count(|o| matches!(o, Output::Samples { .. })) > 20);
    assert_eq!(first_source(&sim), Some(ClockSource::UtcTiming));
}
