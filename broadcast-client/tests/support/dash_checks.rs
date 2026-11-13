//! Invariants every live session run asserts (DESIGN 3.6): no segment requested before its TRUE
//! availability, no gap and no duplicate in the delivered segment sequence.

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::dash_common::synth::{Geo, parse_segment_url};
use super::dash_sim::{DashSim, Logged};

#[derive(Debug, Clone)]
pub struct SegReq {
    pub rep: String,
    pub n: u64,
    pub idx: i64,
    pub send_utc_us: i64,
    pub status: Option<u16>,
}

pub fn segment_requests(sim: &DashSim, geo: Geo) -> Vec<SegReq> {
    sim.log
        .iter()
        .filter_map(|l: &Logged| {
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

pub fn early_requests(reqs: &[SegReq], geo: Geo, tolerance_us: i64) -> Vec<SegReq> {
    reqs.iter()
        .filter(|r| r.send_utc_us + tolerance_us < geo.avail_from_us(r.idx))
        .cloned()
        .collect()
}

fn served(reqs: &[SegReq]) -> BTreeMap<String, Vec<i64>> {
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

pub fn assert_contiguous(reqs: &[SegReq]) {
    for (rep, mut idxs) in served(reqs) {
        idxs.sort_unstable();
        for w in idxs.windows(2) {
            assert_eq!(w[1], w[0] + 1, "rep {rep}: gap or reorder in {idxs:?}");
        }
    }
}

pub fn assert_no_duplicates(reqs: &[SegReq]) {
    let mut counts: BTreeMap<(String, i64), usize> = BTreeMap::new();
    for r in reqs {
        if r.status.is_some_and(|s| (200..300).contains(&s)) {
            *counts.entry((r.rep.clone(), r.idx)).or_insert(0) += 1;
        }
    }
    for ((rep, idx), n) in counts {
        assert_eq!(n, 1, "rep {rep} segment {idx} fetched {n} times");
    }
}

pub fn first_index(reqs: &[SegReq], rep: &str) -> Option<i64> {
    reqs.iter().find(|r| r.rep == rep).map(|r| r.idx)
}
