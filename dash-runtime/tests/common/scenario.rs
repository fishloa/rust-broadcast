//! Shared scaffolding of the live `World` tests.

#![allow(dead_code)]

use super::checks::*;
use super::synth::{Addressing, Geo, MPD_URL, Synth};
use super::world::{MICROS, Shared, Sim, World};
use std::cell::RefCell;
use std::rc::Rc;
use dash_runtime::client::{DashConfig, PullLimits};

pub const AST_US: i64 = 1_700_000_000 * MICROS;
pub const UTC_XSDATE: &str = "http://utc.test/xsdate";
pub const TOLERANCE_US: i64 = 2_000;

pub fn sim_with(synth: Synth, start_after_ast_us: i64, dash: DashConfig) -> (Sim, Geo) {
    let geo = synth.geo();
    let world = World::new(AST_US + start_after_ast_us);
    let sim = Sim::new(MPD_URL, dash, PullLimits::default(), world, Box::new(synth));
    (sim, geo)
}

pub fn with_utc(a: Addressing) -> Synth {
    Synth::new(AST_US, a).with_utc("http-xsdate", UTC_XSDATE)
}

pub fn run_checked(sim: &mut Sim, geo: Geo, secs: i64) -> Vec<SegReq> {
    sim.run_for((secs * MICROS) as u64);
    no_fatal(sim);
    assert!(sim.errors.is_empty(), "{:?}", sim.errors);
    assert!(sim.max_stall_run < 3, "stalled {}", sim.max_stall_run);
    let reqs = segment_requests(sim, geo);
    assert!(!reqs.is_empty());
    assert_contiguous(&reqs);
    assert_no_duplicates(&reqs);
    reqs
}

pub fn sim_shared(
    synth: Synth,
    start_after_ast_us: i64,
    dash: DashConfig,
) -> (Sim, Geo, Rc<RefCell<Synth>>) {
    let geo = synth.geo();
    let handle = Rc::new(RefCell::new(synth));
    let world = World::new(AST_US + start_after_ast_us);
    let sim = Sim::new(
        MPD_URL,
        dash,
        PullLimits::default(),
        world,
        Box::new(Shared(handle.clone())),
    );
    (sim, geo, handle)
}
