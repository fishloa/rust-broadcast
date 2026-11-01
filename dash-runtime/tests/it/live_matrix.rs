//! The failure-injection matrix (the round-8 style): 22 fault kinds against
//! each request type of the live client (MPD refresh, media segment, init,
//! `UTCTiming`), transient and permanent. Every cell asserts: no panic, no
//! hang, a bounded request rate, no segment delivered twice, and either a
//! typed fatal error or continued progress; a transient fault must recover
//! with no loss.

use crate::common;

use std::cell::Cell;
use std::rc::Rc;

use common::checks::*;
use common::scenario::*;
use common::synth::{Addressing, MPD_URL};
use common::world::{HttpReq, HttpResp, Kind, MICROS, Outcome, Sim, World};
use dash_runtime::client::{DashConfig, Failure, FatalKind, Output, PullLimits};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum F {
    S500,
    S503,
    S429,
    S429Huge,
    S404,
    S401,
    S403,
    ConnFail,
    ResetMid,
    ShortBody,
    TruncatedChunk,
    Stall,
    NoResponse,
    Garbage,
    Html,
    Empty,
    Huge,
    Late,
    DupResponse,
    UnknownId,
    ClosedTwice,
    RedirectLoop,
}

const ALL: [F; 22] = [
    F::S500,
    F::S503,
    F::S429,
    F::S429Huge,
    F::S404,
    F::S401,
    F::S403,
    F::ConnFail,
    F::ResetMid,
    F::ShortBody,
    F::TruncatedChunk,
    F::Stall,
    F::NoResponse,
    F::Garbage,
    F::Html,
    F::Empty,
    F::Huge,
    F::Late,
    F::DupResponse,
    F::UnknownId,
    F::ClosedTwice,
    F::RedirectLoop,
];

const RUN_S: i64 = 90;
const WARMUP_S: i64 = 8;
const TRANSIENT_HITS: u32 = 3;
const TRANSIENT_TIMING_HITS: u32 = 1;
const MAX_BURST_PER_S: usize = 40;
const MAX_SUSTAINED_PER_S: f64 = 4.0;
const SUSTAIN_WINDOW_S: u64 = 30;

fn mutate(f: F, resp: HttpResp, redirect: u32) -> Outcome {
    let body = resp.body.clone();
    match f {
        F::ShortBody => {
            let mut r = resp;
            r.body.truncate(body.len() / 2);
            Outcome::Resp(r)
        }
        F::TruncatedChunk => {
            let mut r = resp;
            r.body.truncate(body.len().saturating_sub(7));
            Outcome::Resp(r)
        }
        F::Garbage => Outcome::Resp(HttpResp::ok(vec![0xA5; 400])),
        F::Html => Outcome::Resp(HttpResp::ok(
            b"<html><body>captive portal</body></html>".to_vec(),
        )),
        F::Empty => Outcome::Resp(HttpResp::ok(Vec::new())),
        F::Huge => Outcome::Resp(HttpResp::ok(vec![0u8; 3 * 1024 * 1024])),
        F::RedirectLoop => {
            let mut r = resp;
            r.final_url = Some(format!("http://dash.test/live/loop{redirect}.mpd"));
            Outcome::Resp(r)
        }
        _ => Outcome::Resp(resp),
    }
}

fn outcome_for(f: F, resp: Outcome, redirect: u32) -> Outcome {
    match f {
        F::S500 => Outcome::Resp(HttpResp::status(500)),
        F::S503 => Outcome::Resp(HttpResp::status(503)),
        F::S429 | F::S429Huge => Outcome::Resp(HttpResp::status(429)),
        F::S404 => Outcome::Resp(HttpResp::status(404)),
        F::S401 => Outcome::Resp(HttpResp::status(401)),
        F::S403 => Outcome::Resp(HttpResp::status(403)),
        F::ConnFail | F::ResetMid => Outcome::Fail(Failure::Transport),
        F::Stall | F::NoResponse => Outcome::Hang,
        F::Late | F::DupResponse | F::UnknownId | F::ClosedTwice => resp,
        other => match resp {
            Outcome::Resp(r) => mutate(other, r, redirect),
            o => o,
        },
    }
}

fn build(target: Kind, f: F, permanent: bool) -> (Sim, common::synth::Geo, Rc<Cell<u32>>) {
    let mut synth = with_utc(Addressing::Number);
    synth.send_date = true;
    let geo = synth.geo();
    let mut limits = PullLimits::default();
    limits.max_segment_bytes = 2 * 1024 * 1024;
    limits.limits.max_bytes = 2 * 1024 * 1024;
    let world = World::new(AST_US + 41 * MICROS + 120_000);
    let mut sim = Sim::new(
        MPD_URL,
        DashConfig::default(),
        limits,
        world,
        Box::new(synth),
    );
    sim.give_host_clock();
    let hits = Rc::new(Cell::new(0u32));
    let h = hits.clone();
    let warm = AST_US + (41 + WARMUP_S) * MICROS;
    sim.post = Some(Box::new(
        move |req: &HttpReq, kind: &Kind, at: i64, resp: Outcome| {
            let applies = match kind {
                Kind::Mpd => *kind == target && at > warm,
                Kind::Segment => *kind == target && req.url.contains("/V/") && at > warm,
                Kind::Init => *kind == target && req.url.contains("/V/"),
                Kind::Utc => *kind == target,
                Kind::Index => false,
            };
            let cap = if matches!(f, F::Stall | F::NoResponse | F::Late) {
                TRANSIENT_TIMING_HITS
            } else {
                TRANSIENT_HITS
            };
            if !applies || (!permanent && h.get() >= cap) {
                return resp;
            }
            h.set(h.get() + 1);
            outcome_for(f, resp, h.get())
        },
    ));
    if f == F::Late {
        sim.rtt_fn = Some(Box::new(move |kind, _, sent| {
            (*kind == target && sent > 8 * MICROS as u64).then_some(6 * MICROS as u64)
        }));
    }
    sim.duplicate_responses = f == F::DupResponse;
    sim.double_failures = f == F::ClosedTwice;
    sim.inject_stale = f == F::UnknownId;
    (sim, geo, hits)
}

fn rates(sim: &Sim, kind: Kind) -> (usize, f64) {
    let mut buckets = std::collections::BTreeMap::new();
    for l in sim.log.iter().filter(|l| l.kind == kind) {
        *buckets.entry(l.send_mono_us / 1_000_000).or_insert(0usize) += 1;
    }
    let burst = buckets.values().copied().max().unwrap_or(0);
    let mut sustained = 0.0f64;
    let secs: Vec<u64> = buckets.keys().copied().collect();
    for &s in &secs {
        let n: usize = (s..s + SUSTAIN_WINDOW_S)
            .filter_map(|t| buckets.get(&t))
            .sum();
        sustained = sustained.max(n as f64 / SUSTAIN_WINDOW_S as f64);
    }
    (burst, sustained)
}

fn check_cell(target: Kind, f: F, permanent: bool) {
    let (mut sim, geo, hits) = build(target, f, permanent);
    sim.run_for((RUN_S * MICROS) as u64);
    let label = format!("{target:?} {f:?} permanent={permanent}");
    assert!(hits.get() > 0, "{label}: the fault never applied");
    if std::env::var("MATRIX_DEBUG").is_ok() {
        let mut b = std::collections::BTreeMap::new();
        for l in sim.log.iter().filter(|l| l.kind == Kind::Segment) {
            *b.entry(l.send_mono_us / 1_000_000).or_insert(0usize) += 1;
        }
        eprintln!("{label}: {b:?}");
    }
    assert!(sim.max_stall_run < 3, "{label}: stalled");
    assert!(sim.steps < 200_000, "{label}: {} steps", sim.steps);
    for kind in [Kind::Mpd, Kind::Segment, Kind::Init, Kind::Utc] {
        let (burst, sustained) = rates(&sim, kind);
        assert!(
            burst <= MAX_BURST_PER_S,
            "{label}: {burst} {kind:?} requests in one second"
        );
        assert!(
            sustained <= MAX_SUSTAINED_PER_S,
            "{label}: {sustained} {kind:?} requests/s over 30 s"
        );
    }
    let reqs = segment_requests(&sim, geo);
    let mut accepted = sim.accepted_segments.clone();
    accepted.sort();
    let before = accepted.len();
    accepted.dedup();
    assert_eq!(
        before,
        accepted.len(),
        "{label}: a segment was accepted twice"
    );
    let fatal = sim.fatal().cloned();
    let recent: usize = sim
        .log
        .iter()
        .filter(|l| l.kind == Kind::Segment && l.send_utc_us > AST_US + (41 + RUN_S - 25) * MICROS)
        .count();
    match (&fatal, permanent) {
        (Some(k), _) => assert!(
            matches!(
                k,
                FatalKind::InitFetchFailed { .. }
                    | FatalKind::SegmentFetchFailed { .. }
                    | FatalKind::ClockUnavailable
                    | FatalKind::MpdInvalid { .. }
                    | FatalKind::NonFragmentedMp4 { .. }
            ),
            "{label}: unexpected fatal {k:?}"
        ),
        (None, _) => {
            let retrying = sim
                .log
                .iter()
                .any(|l| l.kind == target && l.send_utc_us > AST_US + (41 + RUN_S - 25) * MICROS);
            assert!(
                recent > 0 || (permanent && retrying),
                "{label}: no progress and no retry in the last 25 s"
            );
        }
    }
    if !permanent && hits.get() > 0 {
        assert!(
            fatal.is_none(),
            "{label}: a transient fault ended the stream: {fatal:?}"
        );
        if sim.count(|o| matches!(o, Output::Notice(dash_runtime::client::Notice::FellBehind))) == 0
        {
            assert_contiguous(&reqs);
        }
        let lost = sim.count(|o| matches!(o, Output::SegmentLost { .. }));
        assert_eq!(lost, 0, "{label}: a transient fault lost a segment");
        assert!(
            sim.count(|o| matches!(o, Output::Samples { .. })) > 10,
            "{label}"
        );
    }
}

#[test]
fn media_segment_faults() {
    for f in ALL {
        for permanent in [false, true] {
            check_cell(Kind::Segment, f, permanent);
        }
    }
}

#[test]
fn mpd_refresh_faults() {
    for f in ALL {
        for permanent in [false, true] {
            check_cell(Kind::Mpd, f, permanent);
        }
    }
}

#[test]
fn init_faults() {
    for f in ALL {
        for permanent in [false, true] {
            check_cell(Kind::Init, f, permanent);
        }
    }
}

#[test]
fn utc_timing_faults() {
    for f in ALL {
        for permanent in [false, true] {
            check_cell(Kind::Utc, f, permanent);
        }
    }
}
