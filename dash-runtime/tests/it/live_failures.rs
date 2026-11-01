//! Early, expired and missing live segments, and the failure cadence
//! (DESIGN 5.4, 23009-1 A.7, RFC 8470) through the `World` simulation.

use crate::common;

use common::checks::*;
use common::scenario::*;
use common::synth::{Addressing, parse_segment_url};
use common::world::{HttpResp, Kind, MICROS, Outcome, Sim};
use dash_runtime::client::{
    DashConfig, Failure, FailureClass, FailureKind, FatalKind, Notice, Output,
};

const START: i64 = 41 * MICROS + 120_000;

fn segment_fault(
    rep: &'static str,
    idx_from: i64,
    idx_to: i64,
    status: u16,
) -> common::world::FaultFn {
    Box::new(move |req, kind, _| {
        if *kind != Kind::Segment {
            return None;
        }
        let (r, n) = parse_segment_url(&req.url)?;
        let idx = n as i64 - 1;
        (r == rep && idx >= idx_from && idx <= idx_to)
            .then(|| Outcome::Resp(HttpResp::status(status)))
    })
}

fn sends_of(sim: &Sim, needle: &str) -> Vec<(u64, Option<u16>)> {
    sim.log
        .iter()
        .filter(|l| l.kind == Kind::Segment && l.url.ends_with(needle))
        .map(|l| (l.send_mono_us, l.status))
        .collect()
}

fn class_count(sim: &Sim, f: impl Fn(&FailureClass) -> bool) -> usize {
    sim.classes.iter().filter(|c| f(c)).count()
}

#[test]
fn a_425_is_never_counted_and_retried_at_250ms_doubling_to_one_second() {
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, DashConfig::default());
    let times = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let t = times.clone();
    sim.fault = Some(Box::new(move |req, kind, _| {
        if *kind == Kind::Segment && req.url.ends_with("/V/25.m4s") && t.get() < 6 {
            t.set(t.get() + 1);
            return Some(Outcome::Resp(HttpResp::status(425)));
        }
        None
    }));
    sim.run_for(40 * MICROS as u64);
    no_fatal(&sim);
    let s = sends_of(&sim, "/V/25.m4s");
    assert_eq!(s.len(), 7, "six refusals then the segment");
    let gaps: Vec<f64> = s
        .windows(2)
        .map(|w| (w[1].0 - w[0].0) as f64 / 1e6)
        .collect();
    let expect = [0.29, 0.54, 1.04, 1.04, 1.04, 1.04];
    for (g, e) in gaps.iter().zip(expect) {
        assert!((g - e).abs() < 0.02, "{gaps:?}");
    }
    assert_eq!(
        class_count(&sim, |c| matches!(c, FailureClass::Retry { .. })),
        0
    );
    assert_eq!(
        class_count(&sim, |c| matches!(c, FailureClass::NotYetAvailable { .. })),
        6
    );
    let reqs = segment_requests(&sim, geo);
    assert_contiguous(&reqs);
}

#[test]
fn a_segment_that_stays_425_past_its_window_is_expired_not_retried_forever() {
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, DashConfig::default());
    sim.fault = Some(segment_fault("V", 24, 24, 425));
    sim.run_for(90 * MICROS as u64);
    no_fatal(&sim);
    let until = geo
        .avail_until_us(24)
        .expect("a bounded availability window");
    let late: Vec<_> = sim
        .log
        .iter()
        .filter(|l| l.kind == Kind::Segment && l.url.ends_with("/V/25.m4s"))
        .filter(|l| l.send_utc_us > until + 2 * MICROS)
        .collect();
    assert!(
        late.is_empty(),
        "{} requests after the window closed",
        late.len()
    );
    assert!(class_count(&sim, |c| matches!(c, FailureClass::Expired)) >= 1);
    let after = segment_requests(&sim, geo)
        .iter()
        .filter(|r| r.rep == "V" && r.idx > 24)
        .count();
    assert!(after > 0, "the lane carried on past the refused segment");
}

#[test]
fn a_404_inside_the_grace_window_is_early_and_after_it_is_missing() {
    let mut dash = DashConfig::default();
    dash.early_grace_us = 1_000_000;
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, dash);
    sim.fault = Some(segment_fault("V", 24, 24, 404));
    sim.run_for(60 * MICROS as u64);
    no_fatal(&sim);
    let avail = geo.avail_from_us(24);
    let sends: Vec<_> = sim
        .log
        .iter()
        .filter(|l| l.kind == Kind::Segment && l.url.ends_with("/V/25.m4s"))
        .collect();
    let early: Vec<_> = sends
        .iter()
        .filter(|l| l.send_utc_us < avail + 1_000_000 - 50_000)
        .collect();
    assert!(early.len() >= 2, "retried inside grace");
    assert!(class_count(&sim, |c| matches!(c, FailureClass::NotYetAvailable { .. })) >= 2);
    assert!(
        class_count(&sim, |c| matches!(
            c,
            FailureClass::Retry {
                kind: FailureKind::Missing,
                ..
            }
        )) >= 1,
        "{:?}",
        sim.classes
    );
}

#[test]
fn a_hole_in_the_window_is_retried_then_skipped_with_a_discontinuity_and_playback_continues() {
    let mut dash = DashConfig::default();
    dash.early_grace_us = 500_000;
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, dash);
    sim.fault = Some(segment_fault("V", 23, 23, 404));
    sim.run_for(80 * MICROS as u64);
    no_fatal(&sim);
    assert_eq!(
        class_count(&sim, |c| matches!(c, FailureClass::SegmentSkipped)),
        1
    );
    assert_eq!(
        sim.count(|o| matches!(
            o,
            Output::SegmentLost {
                status: Some(404),
                ..
            }
        )),
        1
    );
    assert!(sim.count(|o| matches!(o, Output::Discontinuity)) >= 1);
    let reqs = segment_requests(&sim, geo);
    let v: Vec<i64> = reqs
        .iter()
        .filter(|r| r.rep == "V")
        .map(|r| r.idx)
        .collect();
    assert!(
        v.contains(&24) && v.contains(&22),
        "carried on past the hole"
    );
    let attempts = sends_of(&sim, "/V/24.m4s").len();
    assert!(attempts >= 8, "{attempts} attempts before giving up");
}

#[test]
fn the_first_missing_segment_forces_a_clock_resync_and_an_mpd_refresh() {
    let mut dash = DashConfig::default();
    dash.early_grace_us = 500_000;
    dash.utc_resync_us = u64::MAX;
    let mut synth = with_utc(Addressing::Number);
    synth.mup_us = Some(30 * MICROS);
    let (mut sim, _geo) = sim_with(synth, START, dash);
    sim.fault = Some(segment_fault("V", 23, 23, 404));
    sim.run_for(20 * MICROS as u64);
    let utc = sim.requests(Kind::Utc).len();
    assert!(utc >= 2, "initial sample plus the A.7 re-sync, got {utc}");
    let mpd = sim.requests(Kind::Mpd).len();
    assert!(mpd >= 2, "refreshed before the 30 s MUP, got {mpd}");
}

#[test]
fn a_5xx_and_transport_errors_are_counted_with_backoff_and_recover() {
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, DashConfig::default());
    let n = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let c = n.clone();
    sim.fault = Some(Box::new(move |req, kind, _| {
        if *kind == Kind::Segment && req.url.ends_with("/V/25.m4s") && c.get() < 4 {
            c.set(c.get() + 1);
            return Some(match c.get() {
                1 => Outcome::Resp(HttpResp::status(503)),
                2 => Outcome::Fail(Failure::Transport),
                3 => Outcome::Fail(Failure::Timeout),
                _ => Outcome::Resp(HttpResp::status(500)),
            });
        }
        None
    }));
    sim.run_for(40 * MICROS as u64);
    no_fatal(&sim);
    assert_eq!(
        class_count(&sim, |c| matches!(c, FailureClass::Retry { .. })),
        4
    );
    let attempts: Vec<u32> = sim
        .classes
        .iter()
        .filter_map(|c| match c {
            FailureClass::Retry { attempt, .. } => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, vec![1, 2, 3, 4]);
    let reqs = segment_requests(&sim, geo);
    assert_contiguous(&reqs);
    assert_eq!(
        class_count(&sim, |c| matches!(c, FailureClass::SegmentSkipped)),
        0
    );
}

#[test]
fn an_init_that_keeps_failing_is_fatal_for_a_live_presentation_too() {
    let mut limits = dash_runtime::client::PullLimits::default();
    limits.max_init_failures = 3;
    let synth = with_utc(Addressing::Number);
    let world = common::world::World::new(AST_US + START);
    let mut sim = Sim::new(
        common::synth::MPD_URL,
        DashConfig::default(),
        limits,
        world,
        Box::new(synth),
    );
    sim.fault = Some(Box::new(|req, kind, _| {
        (*kind == Kind::Init && req.url.contains("/V/"))
            .then(|| Outcome::Resp(HttpResp::status(500)))
    }));
    sim.run_for(40 * MICROS as u64);
    assert!(
        matches!(sim.fatal(), Some(FatalKind::InitFetchFailed { .. })),
        "{:?}",
        sim.fatal()
    );
}

#[test]
fn a_host_that_slept_past_the_window_skips_to_the_oldest_segment_and_rejoins_after_three() {
    let mut synth = with_utc(Addressing::Number);
    synth.tsb_us = Some(10 * MICROS);
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    sim.run_for(10 * MICROS as u64);
    sim.sleep(60 * MICROS as u64);
    sim.run_for(30 * MICROS as u64);
    no_fatal(&sim);
    assert!(sim.count(|o| matches!(o, Output::Notice(Notice::FellBehind))) >= 1);
    assert!(sim.count(|o| matches!(o, Output::Discontinuity)) >= 1);
    let reqs = segment_requests(&sim, geo);
    let after: Vec<_> = reqs
        .iter()
        .filter(|r| r.send_utc_us > AST_US + START + 70 * MICROS && r.rep == "V")
        .collect();
    assert!(!after.is_empty(), "playback resumed");
    let now = sim.world.true_utc();
    let edge = geo.complete_count(now) - 1;
    let last = after.last().unwrap().idx;
    assert!(
        edge - last <= 3,
        "back near the live edge: {last} vs {edge}"
    );
    assert!(early_requests(&reqs, geo, TOLERANCE_US).is_empty());
}

#[test]
fn a_response_that_arrives_after_its_segment_left_the_window_is_expired_not_counted() {
    let mut synth = with_utc(Addressing::Number);
    synth.tsb_us = Some(10 * MICROS);
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    sim.rtt_fn = Some(Box::new(|kind, url, sent| {
        (*kind == Kind::Segment && url.contains("/V/") && sent < 100_000)
            .then_some(40 * MICROS as u64)
    }));
    sim.run_for(80 * MICROS as u64);
    no_fatal(&sim);
    assert!(
        class_count(&sim, |c| matches!(c, FailureClass::Expired)) >= 1,
        "{:?}",
        sim.classes
    );
    assert_eq!(
        class_count(&sim, |c| matches!(c, FailureClass::Retry { .. })),
        0
    );
    assert!(sim.count(|o| matches!(o, Output::Notice(Notice::FellBehind))) >= 1);
    let reqs = segment_requests(&sim, geo);
    assert!(sim.count(|o| matches!(o, Output::Samples { .. })) > 10);
    assert!(early_requests(&reqs, geo, TOLERANCE_US).is_empty());
}

#[test]
fn garbage_and_truncated_segment_bodies_are_counted_not_delivered() {
    let (mut sim, geo) = sim_with(with_utc(Addressing::Number), START, DashConfig::default());
    sim.fault = Some(Box::new(|req, kind, _| {
        if *kind == Kind::Segment && req.url.ends_with("/A/25.m4s") {
            return Some(Outcome::Resp(HttpResp::ok(
                b"<html>captive</html>".to_vec(),
            )));
        }
        None
    }));
    sim.run_for(80 * MICROS as u64);
    no_fatal(&sim);
    assert!(
        class_count(&sim, |c| matches!(
            c,
            FailureClass::Retry {
                kind: FailureKind::Invalid,
                ..
            }
        )) >= 1
    );
    assert_eq!(
        class_count(&sim, |c| matches!(c, FailureClass::SegmentSkipped)),
        1
    );
    let _ = geo;
}

#[test]
fn l4_early_bookkeeping_does_not_leak_after_give_ups() {
    let mut dash = DashConfig::default();
    dash.early_grace_us = 500_000;
    let (mut sim, _geo) = sim_with(with_utc(Addressing::Number), START, dash);
    sim.fault = Some(segment_fault("V", 23, 23, 404));
    sim.run_for(80 * MICROS as u64);
    no_fatal(&sim);
    assert_eq!(sim.client.state_size().live, 0);
}
