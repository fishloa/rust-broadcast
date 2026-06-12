//! Fixture smoke test: feed real broadcast captures through the monitor.
//!
//! ## m6-single.ts
//!
//! This fixture was captured for section-parsing validation, not CC continuity.
//! The PES PIDs (0x0082, 0x0083, 0x0084, …) carry genuine CC discontinuities
//! (the continuity counter values do not increment sequentially — e.g. 15→14→3).
//! Indicator 1.4 (`ContinuityCountError`) therefore fires on these PIDs.
//! The test asserts **zero non-CC Priority-1 events** (sync, PAT, PMT, PID)
//! and documents the expected CC errors.
//!
//! ## tnt-5w-12732v-isi6-10s.ts
//!
//! This is a T2-MI outer stream whose PID layout does not resemble normal
//! DVB SI, so P1 events on it are expected/uninteresting; the test just
//! verifies the monitor runs without panicking.

use core::time::Duration;
use std::fs::File;
use std::io::Read;

use dvb_conformance::{ConformanceMonitor, Indicator, Priority};

const TS_PACKET_SIZE: usize = 188;

/// Inter-packet interval for synthetic timestamps. At ~38 Mbit/s a 188-byte
/// packet takes ~40 µs; using 40 µs makes a 10 s capture span ~10 s of
/// simulated wall-clock time. Presence timers (500 ms / 5 s) will not trip
/// on these short captures under this model.
const INTER_PACKET_US: u64 = 40;

fn read_fixture(name: &str) -> Vec<u8> {
    let base = concat!(env!("CARGO_MANIFEST_DIR"), "/../dvb-si/tests/fixtures");
    let path = format!("{}/{}", base, name);
    let mut f = File::open(&path).unwrap_or_else(|e| panic!("cannot open {}: {}", path, e));
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    buf
}

fn run_monitor_on_fixture(name: &str) -> Vec<dvb_conformance::ConformanceEvent> {
    let data = read_fixture(name);
    let mut monitor = ConformanceMonitor::new();
    let mut all_events = Vec::new();

    let n_packets = data.len() / TS_PACKET_SIZE;
    for i in 0..n_packets {
        let start = i * TS_PACKET_SIZE;
        let end = start + TS_PACKET_SIZE;
        if end > data.len() {
            break;
        }
        let t = Duration::from_micros(i as u64 * INTER_PACKET_US);
        let events = monitor.feed(&data[start..end], t);
        all_events.extend(events.to_vec());
    }
    all_events
}

#[test]
fn m6_single_no_non_cc_priority1_events() {
    let events = run_monitor_on_fixture("m6-single.ts");

    let non_cc_p1: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(e.priority, Priority::First) && e.indicator != Indicator::ContinuityCountError
        })
        .collect();

    if !non_cc_p1.is_empty() {
        for e in &non_cc_p1 {
            eprintln!(
                "non-CC P1 event on m6-single.ts: {:?} pid={:?} detail={}",
                e.indicator, e.pid, e.detail
            );
        }
        panic!(
            "m6-single.ts raised {} non-CC Priority-1 event(s) — investigate",
            non_cc_p1.len()
        );
    }

    // CC errors ARE expected: the m6-single.ts fixture carries PES PIDs with
    // genuine CC discontinuities (the capture predates CC-continuity testing).
    let cc_count = events
        .iter()
        .filter(|e| e.indicator == Indicator::ContinuityCountError)
        .count();
    assert!(
        cc_count > 0,
        "m6-single.ts is known to have CC discontinuities — expected some ContinuityCountError events"
    );
}

#[test]
fn tnt_fixture_events_are_documented() {
    // The tnt fixture is a T2-MI outer stream — its PID layout does not look
    // like a normal DVB SI multiplex. P1 events (especially PAT/PMT absence
    // and CC errors) are expected. This test just verifies the monitor runs
    // without panicking and records the event count for documentation.
    let events = run_monitor_on_fixture("tnt-5w-12732v-isi6-10s.ts");
    let p1_count = events
        .iter()
        .filter(|e| matches!(e.priority, Priority::First))
        .count();
    eprintln!(
        "tnt fixture: {} total events, {} P1 (expected for T2-MI outer stream)",
        events.len(),
        p1_count
    );
}
