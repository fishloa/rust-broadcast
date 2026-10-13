//! Hostile-count sweep over the committed MP4 fixtures (#1130).
//!
//! For every ISO-BMFF box in every committed `.mp4`/`.m4s` fixture, overwrite
//! each 4-byte word in the 16 bytes after the box's fourcc (the FullBox
//! version/flags, `entry_count`, `sample_count`, `data_offset` ... region)
//! with an inflated value, and demux the result through `Fmp4Demux` and
//! `ProgressiveDemux`. Each parse runs under a per-thread live-byte cap, so a
//! declared count that reaches the allocator as `count * size_of::<Entry>()`
//! aborts the test binary instead of passing silently.
//!
//! This is the sweep that found the zero-width `trun` hole (a 20-byte `trun`
//! declaring `0x1000_0000` samples asked for a 10 GB `Vec` out of
//! `ll_chunked.mp4`; see `trun_zero_width_alloc_bound.rs` for the pinned
//! repro). It checks nothing about the *result* — only that no hostile count
//! turns into an unbounded allocation or loop.

use std::path::{Path, PathBuf};

use broadcast_common::Unpackage;
use test_alloc::ThreadCapped;
use transmux::{Fmp4Demux, ProgressiveDemux};

/// 128 MiB live bytes per thread: far above anything these <= 100 KB inputs
/// legitimately need, far below what an inflated count asks for.
#[global_allocator]
static ALLOC: ThreadCapped<{ 128 << 20 }> = ThreadCapped::new();

const INFLATED: [[u8; 4]; 2] = [[0xFF, 0xFF, 0xFF, 0xFF], [0x10, 0x00, 0x00, 0x00]];
/// Bytes after each fourcc whose 4-byte words are overwritten.
const REGION: usize = 16;
/// Skip fixtures larger than this (keeps the debug-build runtime low).
const MAX_FIXTURE_LEN: u64 = 100_000;

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "mp4" || x == "m4s")
            && std::fs::metadata(&p).is_ok_and(|m| m.len() <= MAX_FIXTURE_LEN)
        {
            out.push(p);
        }
    }
}

/// Offsets of the first byte after every plausible fourcc: a 4-byte ASCII
/// lowercase/digit word preceded by a plausible 32-bit box size.
fn after_fourcc(data: &[u8]) -> Vec<usize> {
    let mut v = Vec::new();
    for i in 4..data.len().saturating_sub(8) {
        let cc = &data[i..i + 4];
        let size =
            u32::from_be_bytes([data[i - 4], data[i - 3], data[i - 2], data[i - 1]]) as usize;
        if cc
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && cc[0].is_ascii_lowercase()
            && size >= 8
            && size <= data.len()
        {
            v.push(i + 4);
        }
    }
    v
}

#[test]
fn inflated_counts_never_become_unbounded_allocations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/mp4");
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    assert!(files.len() >= 10, "only {} fixtures found", files.len());

    let mut parses = 0usize;
    for path in &files {
        let data = std::fs::read(path).unwrap();
        for start in after_fourcc(&data) {
            for word in (0..REGION).step_by(4) {
                let at = start + word;
                if at + 4 > data.len() {
                    break;
                }
                for pat in INFLATED {
                    let mut m = data.clone();
                    m[at..at + 4].copy_from_slice(&pat);
                    let _ = Fmp4Demux::new().unpackage(&m);
                    if let Ok(mut d) = ProgressiveDemux::new(1 << 30) {
                        let _ = d.unpackage(&m);
                    }
                    parses += 1;
                }
            }
        }
    }
    eprintln!(
        "hostile_count_sweep: {} fixtures, {parses} mutated parses",
        files.len()
    );
    assert!(parses > 1_000, "sweep ran only {parses} parses");
}
