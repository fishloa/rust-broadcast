//! Audit MD-O2 (issue #1112): `CcAnomalyCheck` used to copy every
//! payload-bearing packet into a fresh 188-byte `Vec` ("last packet seen on
//! this PID"), one heap allocation per packet. The copy is now a fixed
//! array, so a clean stream costs O(PIDs) allocations, not O(packets).

use media_doctor::{CcAnomalyCheck, Diagnostic, Report};
use test_alloc::ThreadCounting;

#[global_allocator]
static ALLOC: ThreadCounting = ThreadCounting::new();

const PACKETS: usize = 20_000;

/// A clean one-PID stream: payload-only packets, CC counting 0..=15 with no gaps.
fn clean_stream() -> Vec<u8> {
    let mut ts = Vec::with_capacity(PACKETS * 188);
    for i in 0..PACKETS {
        let mut p = [0xAAu8; 188];
        p[0] = 0x47;
        p[1] = 0x01; // PID 0x0100, no PUSI
        p[2] = 0x00;
        p[3] = 0x10 | (i % 16) as u8; // payload only, CC
        // Vary the payload so no packet is a byte-identical duplicate.
        p[4] = (i >> 8) as u8;
        p[5] = i as u8;
        ts.extend_from_slice(&p);
    }
    ts
}

#[test]
fn clean_stream_does_not_allocate_per_packet() {
    let ts = clean_stream();
    let mut report = Report::new();
    let (_, snap) = ThreadCounting::measure(|| CcAnomalyCheck.run(&ts, &mut report));
    assert!(report.findings().is_empty(), "stream must be clean");
    assert!(
        snap.allocs < 100,
        "{} allocations for {PACKETS} packets: one per packet again",
        snap.allocs
    );
}
