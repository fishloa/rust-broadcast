//! Multi-Period live presentations through the `World` simulation: Periods
//! announced ahead, added by a refresh and removed once expired (DESIGN 5.5).

mod common;

use common::checks::*;
use common::scenario::*;
use common::synth::Addressing;
use common::world::{Kind, MICROS, Sim};
use dash_runtime::client::{DashConfig, Output};

const PERIOD_STARTS: [i64; 3] = [0, 40 * MICROS, 80 * MICROS];

fn multi(a: Addressing, ahead: i64) -> common::synth::Synth {
    let mut s = with_utc(a);
    s.period_starts_us = PERIOD_STARTS.to_vec();
    s.announce_ahead_us = ahead;
    s.tsb_us = Some(20 * MICROS);
    s
}

fn period_changes(sim: &Sim) -> Vec<(String, bool)> {
    sim.outputs
        .iter()
        .filter_map(|o| match o {
            Output::PeriodChanged { id, continuous, .. } => {
                Some((id.clone().unwrap_or_default(), *continuous))
            }
            _ => None,
        })
        .collect()
}

fn assert_across_boundaries(reqs: &[SegReq]) {
    for rep in ["V", "A"] {
        let mut by_period: Vec<(usize, Vec<i64>)> = Vec::new();
        for r in reqs.iter().filter(|r| r.rep.starts_with(rep)) {
            let p: usize = r.rep[1..].parse().unwrap();
            if !by_period.iter().any(|(q, _)| *q == p) {
                by_period.push((p, Vec::new()));
            }
            let v = &mut by_period.iter_mut().find(|(q, _)| *q == p).unwrap().1;
            if !v.contains(&r.idx) {
                v.push(r.idx);
            }
        }
        by_period.sort_by_key(|(p, _)| *p);
        let mut all: Vec<i64> = Vec::new();
        for (_, v) in &by_period {
            all.extend(v);
        }
        for w in all.windows(2) {
            assert_eq!(
                w[1],
                w[0] + 1,
                "{rep}: gap or repeat across Periods in {all:?}"
            );
        }
    }
}

#[test]
fn playing_through_two_boundaries_changes_period_twice_with_a_discontinuity_each() {
    for a in [
        Addressing::Number,
        Addressing::TimelineNumber,
        Addressing::TimelineTime,
    ] {
        let (mut sim, geo) = sim_with(multi(a, 6 * MICROS), 25 * MICROS, DashConfig::default());
        sim.run_for(80 * MICROS as u64);
        no_fatal(&sim);
        assert!(sim.errors.is_empty(), "{a:?}: {:?}", sim.errors);
        let reqs = segment_requests(&sim, geo);
        assert_no_duplicates(&reqs);
        assert_across_boundaries(&reqs);
        assert!(early_requests(&reqs, geo, TOLERANCE_US).is_empty(), "{a:?}");
        let changes = period_changes(&sim);
        assert_eq!(
            changes.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            vec!["P0", "P1", "P2"],
            "{a:?}"
        );
        assert!(
            changes.iter().all(|c| !c.1),
            "unsignalled boundaries are not continuous"
        );
        assert_eq!(
            sim.count(|o| matches!(o, Output::Discontinuity)),
            2,
            "{a:?}"
        );
        let inits = sim.requests(Kind::Init).len();
        assert_eq!(inits, 6, "{a:?}: one init per Representation per Period");
    }
}

#[test]
fn a_period_announced_ahead_gets_its_init_before_its_first_segment() {
    let (mut sim, geo) = sim_with(
        multi(Addressing::Number, 10 * MICROS),
        25 * MICROS,
        DashConfig::default(),
    );
    sim.run_for(30 * MICROS as u64);
    no_fatal(&sim);
    let p1_init = sim
        .requests(Kind::Init)
        .into_iter()
        .find(|l| l.url.contains("/V1/"))
        .expect("Period 1 init fetched");
    let first_p1_segment = sim
        .requests(Kind::Segment)
        .into_iter()
        .find(|l| l.url.contains("/V1/"))
        .expect("Period 1 segment fetched");
    assert!(p1_init.send_utc_us <= first_p1_segment.send_utc_us);
    assert!(
        p1_init.send_utc_us >= AST_US + 40 * MICROS - TOLERANCE_US,
        "the init exists from AST + PeriodStart (5.3.9.5.2)"
    );
    let reqs = segment_requests(&sim, geo);
    assert!(early_requests(&reqs, geo, TOLERANCE_US).is_empty());
}

#[test]
fn expired_periods_dropping_out_of_the_mpd_do_not_disturb_playback() {
    let (mut sim, geo) = sim_with(
        multi(Addressing::TimelineNumber, 6 * MICROS),
        55 * MICROS,
        DashConfig::default(),
    );
    sim.run_for(70 * MICROS as u64);
    no_fatal(&sim);
    let reqs = segment_requests(&sim, geo);
    assert_no_duplicates(&reqs);
    assert_across_boundaries(&reqs);
    assert_eq!(
        sim.count(|o| matches!(o, Output::Discontinuity)),
        1,
        "only the boundary into Period 2, nothing for the removal of Period 0"
    );
}

#[test]
fn a_period_that_appears_with_a_refresh_closes_the_previous_open_ended_one() {
    let (mut sim, geo) = sim_with(
        multi(Addressing::Number, 4 * MICROS),
        25 * MICROS,
        DashConfig::default(),
    );
    sim.run_for(40 * MICROS as u64);
    no_fatal(&sim);
    let reqs = segment_requests(&sim, geo);
    assert_across_boundaries(&reqs);
    assert!(period_changes(&sim).len() >= 2);
}
