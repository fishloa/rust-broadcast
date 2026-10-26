//! Invariants every `World` run asserts (DESIGN 3.6): no segment requested
//! before its TRUE availability, no gap and no duplicate in the delivered
//! segment sequence, a join within the presentation delay of the live edge.

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::synth::{Geo, parse_segment_url};
use super::world::{Kind, Sim};
use dash_runtime::client::Output;

#[derive(Debug, Clone)]
pub struct SegReq {
    pub rep: String,
    pub n: u64,
    pub idx: i64,
    pub send_utc_us: i64,
    pub status: Option<u16>,
}

pub fn segment_requests(sim: &Sim, geo: Geo) -> Vec<SegReq> {
    sim.log
        .iter()
        .filter(|l| l.kind == Kind::Segment)
        .filter_map(|l| {
            let (rep, n) = parse_segment_url(&l.url)?;
            let idx = geo.index_of(&rep, n)?;
            Some(SegReq {
                rep,
                n,
                idx,
                send_utc_us: l.send_utc_us,
                status: l.status,
            })
        })
        .collect()
}

/// Requests sent more than `tolerance_us` before their true availability.
pub fn early_requests(reqs: &[SegReq], geo: Geo, tolerance_us: i64) -> Vec<SegReq> {
    reqs.iter()
        .filter(|r| r.send_utc_us + tolerance_us < geo.avail_from_us(r.idx))
        .cloned()
        .collect()
}

/// Successfully served segment indices per rep, in request order, each once.
pub fn delivered(reqs: &[SegReq]) -> BTreeMap<String, Vec<i64>> {
    let mut out: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for r in reqs {
        if r.status.is_some_and(|s| (200..300).contains(&s)) {
            let v = out.entry(r.rep.clone()).or_default();
            if !v.contains(&r.idx) {
                v.push(r.idx);
            }
        }
    }
    out
}

/// Assert each rep's served indices form one run (no gap, no repeat); the wire
/// order is not the delivery order, retries and the in-flight window reorder it.
pub fn assert_contiguous(reqs: &[SegReq]) {
    for (rep, mut idxs) in delivered(reqs) {
        idxs.sort_unstable();
        for w in idxs.windows(2) {
            assert_eq!(
                w[1],
                w[0] + 1,
                "rep {rep}: gap or reorder in {idxs:?}"
            );
        }
    }
}

/// How many times each index of each rep was successfully served.
pub fn served_counts(reqs: &[SegReq]) -> BTreeMap<(String, i64), usize> {
    let mut m = BTreeMap::new();
    for r in reqs {
        if r.status.is_some_and(|s| (200..300).contains(&s)) {
            *m.entry((r.rep.clone(), r.idx)).or_insert(0) += 1;
        }
    }
    m
}

pub fn assert_no_duplicates(reqs: &[SegReq]) {
    for ((rep, idx), n) in served_counts(reqs) {
        assert_eq!(n, 1, "rep {rep} segment {idx} fetched {n} times");
    }
}

/// The first requested index of `rep`.
pub fn first_index(reqs: &[SegReq], rep: &str) -> Option<i64> {
    reqs.iter().find(|r| r.rep == rep).map(|r| r.idx)
}

pub fn no_fatal(sim: &Sim) {
    assert!(sim.fatal().is_none(), "fatal: {:?}", sim.fatal());
}

pub fn clock_sources(sim: &Sim) -> Vec<(dash_runtime::timing::clock::ClockSource, u64)> {
    sim.outputs
        .iter()
        .filter_map(|o| match o {
            Output::ClockSynced {
                source,
                uncertainty_us,
            } => Some((*source, *uncertainty_us)),
            _ => None,
        })
        .collect()
}
