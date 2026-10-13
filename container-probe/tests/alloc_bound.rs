//! Pins the allocation claim in the crate docs (audit r01-O7): a probe that
//! finds no candidate allocates nothing; one that matches allocates the
//! internal candidate `Vec` exactly once.

use container_probe::{Probe, probe};
use test_alloc::ThreadCounting;

#[global_allocator]
static A: ThreadCounting = ThreadCounting::new();

#[test]
fn no_candidate_probe_allocates_nothing() {
    let junk = [0xFFu8; 4096];
    let (p, snap) = ThreadCounting::measure(|| probe(&junk));
    assert!(matches!(p, Probe::Unknown), "got {p:?}");
    assert_eq!(snap.allocs, 0, "Unknown probe allocated");

    let short = [0u8; 8];
    let (p, snap) = ThreadCounting::measure(|| probe(&short));
    assert!(matches!(p, Probe::Insufficient { .. }), "got {p:?}");
    assert_eq!(snap.allocs, 0, "Insufficient probe allocated");
}

#[test]
fn matching_probe_allocates_the_candidate_vec_once() {
    let mut ogg = b"OggS".to_vec();
    ogg.resize(64, 0);
    let (p, snap) = ThreadCounting::measure(|| probe(&ogg));
    assert!(matches!(p, Probe::Identified { .. }), "got {p:?}");
    assert_eq!(snap.allocs, 1, "one candidate Vec expected");
}
