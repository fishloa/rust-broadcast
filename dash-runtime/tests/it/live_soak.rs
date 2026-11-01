//! A no-panic soak: many randomised sessions (addressing, Periods, clock
//! sources and errors, drift, faults, sleeps, host clock calls) each checked
//! against the invariants that must hold whatever the network does.

use crate::common;

use std::cell::RefCell;
use std::rc::Rc;

use common::Rng;
use common::scenario::*;
use common::synth::{Addressing, MPD_URL, Synth, UtcMode};
use common::world::{HttpResp, Kind, MICROS, Outcome, Sim, World};
use dash_runtime::client::{DashConfig, FatalKind, PullLimits};

const SESSIONS: u64 = 160;

fn session(seed: u64) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let a = [
        Addressing::Number,
        Addressing::TimelineNumber,
        Addressing::TimelineTime,
    ][rng.below(3) as usize];
    let ast = if a == Addressing::Number { AST_US } else { 0 };
    let mut synth = Synth::new(ast, a);
    synth.tsb_us = [Some(8), Some(20), Some(60), None][rng.below(4) as usize].map(|s| s * MICROS);
    synth.mup_us = [Some(0), Some(2), Some(5), None][rng.below(4) as usize].map(|s| s * MICROS);
    synth.spd_us = [None, Some(4), Some(8)][rng.below(3) as usize].map(|s| s * MICROS);
    synth.window = rng.range(2, 12);
    synth.early_status = [404, 425][rng.below(2) as usize];
    synth.send_date = rng.below(2) == 0;
    synth.send_etag = rng.below(2) == 0;
    synth.per_segment_s = rng.below(2) == 0;
    let utc = rng.below(5);
    synth = match utc {
        0 => synth,
        1 => synth.with_utc("http-xsdate", UTC_XSDATE),
        2 => synth.with_utc("http-head", "http://utc.test/head"),
        3 => synth.with_utc("http-iso", "http://utc.test/iso"),
        _ => {
            synth.utc_direct = true;
            synth
        }
    };
    synth.utc.skew_us = (rng.below(7) as i64 - 3) * MICROS;
    synth.utc.mode = [
        UtcMode::Ok,
        UtcMode::Ok,
        UtcMode::Garbage,
        UtcMode::Hang,
        UtcMode::Fail,
    ][rng.below(5) as usize];
    if rng.below(4) == 0 {
        synth.utc.head_status = Some(405);
    }
    if rng.below(3) == 0 {
        let n = rng.range(2, 5) as i64;
        synth.period_starts_us = (0..n).map(|i| i * 40 * MICROS).collect();
        synth.announce_ahead_us = rng.range(0, 10) as i64 * MICROS;
    }
    let geo = synth.geo();
    let start = rng.range(30, 90) as i64 * MICROS + rng.below(1_000_000) as i64;
    let mut dash = DashConfig::default();
    dash.utc_resync_us = [MICROS as u64 * 20, 600 * MICROS as u64, u64::MAX][rng.below(3) as usize];
    dash.early_grace_us = [0, MICROS as u64, 3 * MICROS as u64][rng.below(3) as usize];
    dash.use_availability_time_offset = rng.below(5) == 0;
    dash.availability_margin_us = [0, 0, 100_000][rng.below(3) as usize];
    let mut limits = PullLimits::default();
    limits.max_inflight_segments = rng.range(1, 6) as usize;
    limits.max_segment_failures = [0, 1, 3, 8][rng.below(4) as usize];
    let handle = Rc::new(RefCell::new(synth));
    let mut world = World::new(AST_US + start);
    world.host_skew_us = (rng.below(21) as i64 - 10) * MICROS / 2;
    world.host_drift_ppm = rng.below(401) as i64 - 200;
    let mut sim = Sim::new(
        MPD_URL,
        dash,
        limits,
        world,
        Box::new(common::world::Shared(handle.clone())),
    );
    sim.keep_samples = false;
    sim.trace_stall = std::env::var("SOAK_TRACE").is_ok();
    sim.rtt_us = rng.range(2_000, 600_000);
    if rng.below(2) == 0 {
        sim.give_host_clock();
    }
    let fault_rate = rng.below(40);
    let mut frng = Rng(rng.next() | 1);
    sim.post = Some(Box::new(move |_req, kind, _at, resp| {
        if frng.below(100) >= fault_rate || *kind == Kind::Index {
            return resp;
        }
        match frng.below(8) {
            0 => Outcome::Resp(HttpResp::status(404)),
            1 => Outcome::Resp(HttpResp::status(425)),
            2 => Outcome::Resp(HttpResp::status(410)),
            3 => Outcome::Resp(HttpResp::status(500)),
            4 => Outcome::Fail(dash_runtime::client::Failure::Transport),
            5 => Outcome::Resp(HttpResp::ok(b"garbage".to_vec())),
            6 => Outcome::Hang,
            _ => resp,
        }
    }));
    let mut churn = Rng(rng.next() | 1);
    for _ in 0..rng.range(3, 10) {
        sim.run_for(churn.range(5, 60) * MICROS as u64);
        match churn.below(5) {
            0 => sim.sleep(churn.range(1, 90) * MICROS as u64),
            1 => {
                sim.world.host_step_us = (churn.below(61) as i64 - 30) * MICROS;
                sim.give_host_clock();
            }
            2 => sim.give_host_clock(),
            3 => {
                let mut s = handle.borrow_mut();
                if churn.below(2) == 0 {
                    s.tsb_us = [Some(8 * MICROS), Some(30 * MICROS)][churn.below(2) as usize];
                } else {
                    s.send_date = !s.send_date;
                }
            }
            _ => {}
        }
        if sim.client.presentation_ended() || sim.fatal().is_some() {
            break;
        }
    }
    let label = format!("seed {seed}");
    assert!(
        sim.max_stall_run < 4,
        "{label}: stalled {}",
        sim.max_stall_run
    );
    let mut accepted = sim.accepted_segments.clone();
    accepted.sort();
    let n = accepted.len();
    accepted.dedup();
    assert_eq!(n, accepted.len(), "{label}: a segment was accepted twice");
    let st = sim.client.state_size();
    assert!(
        st.requests <= 64 && st.inits <= 64 && st.periods <= 8,
        "{label}: {st:?}"
    );
    if let Some(k) = sim.fatal() {
        assert!(
            matches!(
                k,
                FatalKind::ClockUnavailable
                    | FatalKind::InitFetchFailed { .. }
                    | FatalKind::SegmentFetchFailed { .. }
                    | FatalKind::MpdInvalid { .. }
                    | FatalKind::NonFragmentedMp4 { .. }
                    | FatalKind::UnsupportedAddressing { .. }
            ),
            "{label}: {k:?}"
        );
    }
    let _ = geo;
}

const UTC_XSDATE: &str = "http://utc.test/xsdate";

#[test]
fn one_session_for_debugging() {
    if let Ok(v) = std::env::var("SOAK_SEED") {
        session(v.parse().unwrap());
    }
}

#[test]
fn randomised_sessions_never_panic_and_keep_their_invariants() {
    let n = std::env::var("SOAK_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SESSIONS);
    for seed in 0..n {
        session(seed);
    }
}
