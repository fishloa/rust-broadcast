#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use dash_runtime::address::{PlanOptions, PlannedRepresentation, plan_representation};
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::RepRef;
use url::Url;

pub struct Loaded {
    pub path: PathBuf,
    pub mpd: Mpd,
    pub url: Url,
}

pub fn load(rel: &str) -> Loaded {
    let path = test_fixtures::path(rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mpd = Mpd::parse(&bytes).unwrap_or_else(|e| panic!("{rel}: {e}"));
    let path = std::fs::canonicalize(&path).unwrap();
    let url = Url::from_file_path(&path).unwrap();
    Loaded { path, mpd, url }
}

impl Loaded {
    pub fn reps(&self) -> Vec<RepRef> {
        let mut out = Vec::new();
        for (pi, p) in self.mpd.periods.iter().enumerate() {
            for (si, s) in p.adaptation_sets.iter().enumerate() {
                for ri in 0..s.representations.len() {
                    out.push(RepRef::at(pi, si, ri));
                }
            }
        }
        out
    }

    pub fn plan(&self, at: RepRef) -> PlannedRepresentation {
        plan_representation(&self.mpd, at, &PlanOptions::default())
            .unwrap_or_else(|e| panic!("{at:?}: {e}"))
    }

    pub fn file_of(&self, planned: &PlannedRepresentation, reference: &str) -> PathBuf {
        let resolved = planned.candidates[0]
            .resolve(Some(&self.url), reference)
            .unwrap_or_else(|| panic!("cannot resolve {reference:?}"));
        Url::parse(&resolved)
            .unwrap()
            .to_file_path()
            .unwrap_or_else(|()| panic!("not a file URL: {resolved}"))
    }
}

impl Loaded {
    /// The bytes of the single file a SegmentBase Representation lives in.
    pub fn base_file_bytes(&self, at: RepRef) -> Vec<u8> {
        let (period, set, rep) = at.resolve(&self.mpd).unwrap();
        let cands = dash_runtime::mpd::urls::representation_candidates(&self.mpd, period, set, rep);
        let resolved = cands[0].resolve(Some(&self.url), "").unwrap();
        let path = Url::parse(&resolved).unwrap().to_file_path().unwrap();
        std::fs::read(path).unwrap()
    }
}

/// `file name -> first tfdt` lines of the MP4Box block of `<dir>/oracle.txt`.
pub fn oracle_tfdt(dir: &str) -> HashMap<String, Vec<u64>> {
    let text = test_fixtures::read_to_string(&format!("{dir}/oracle.txt"));
    let mut out = HashMap::new();
    let mut in_block = false;
    for line in text.lines() {
        if line.starts_with("## MP4Box -diso") {
            in_block = true;
            continue;
        }
        if line.starts_with("## ") {
            in_block = false;
        }
        if !in_block {
            continue;
        }
        let Some((name, rest)) = line.trim().split_once(": ") else {
            continue;
        };
        let Some(tf) = rest.split("tfdt=").nth(1) else {
            continue;
        };
        let list = tf.split_whitespace().next().unwrap_or_default();
        let vals: Vec<u64> = list
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        out.insert(name.to_owned(), vals);
    }
    out
}

/// `file name -> sidx earliest_presentation_time` of the MP4Box block.
pub fn oracle_epct(dir: &str) -> HashMap<String, u64> {
    let text = test_fixtures::read_to_string(&format!("{dir}/oracle.txt"));
    let mut out = HashMap::new();
    for line in text.lines() {
        let Some((name, rest)) = line.trim().split_once(": ") else {
            continue;
        };
        if let Some(v) = rest.split("earliest_presentation_time=").nth(1) {
            let digits: String = v.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = digits.parse() {
                out.insert(name.to_owned(), n);
            }
        }
    }
    out
}

/// xorshift64*: deterministic, dependency-free randomness for property tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }
}
pub mod loopback;
pub mod scan;
