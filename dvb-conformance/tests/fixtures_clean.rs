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
//! `tsanalyze --normalized` on this fixture shows `pcrpids=0` and `pcr=0` on
//! every PID: it carries **no PCR at all** (an SI-only extract). This test's
//! packet arrival times are therefore not real — they're invented by the
//! harness's 40 µs/packet constant-bitrate model (`run_monitor_on_fixture`
//! below), the only clock a PCR-less capture leaves available. Under that
//! invented clock, PID 0x0000 (PAT) — which repeats 32 times across the
//! fixture's 1264 packets — arrives once every ~1.6 ms, and TBsys (512
//! bytes, 125 000 bytes/s leak, ISO/IEC 13818-1 §2.4.2.3) cannot drain fast
//! enough to keep up. `BufferError` (indicator 3.3) is excluded from the
//! zero-T-STD-events assertion for exactly that reason: TBsys occupancy
//! cannot be judged on a fixture whose timing was invented, not captured.
//! `EmptyBufferError`/`DataDelayError` are not excluded, since those depend
//! only on TBsys draining regularly, which it does even under this clock.
//!
//! ## tnt-5w-12732v-isi6-10s.ts
//!
//! This is a T2-MI outer stream whose PID layout does not resemble normal
//! DVB SI, so P1 events on it are expected/uninteresting; the test just
//! verifies the monitor runs without panicking.

use core::time::Duration;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use dvb_conformance::{ConformanceMonitor, Indicator, Priority};

const TS_PACKET_SIZE: usize = 188;

/// Inter-packet interval for synthetic timestamps. At ~38 Mbit/s a 188-byte
/// packet takes ~40 µs; using 40 µs makes a 10 s capture span ~10 s of
/// simulated wall-clock time. Presence timers (500 ms / 5 s) will not trip
/// on these short captures under this model.
const INTER_PACKET_US: u64 = 40;

fn read_fixture(name: &str) -> Vec<u8> {
    let base = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures");
    let subdir = if name == "m6-single.ts" {
        "/ts/"
    } else {
        "/dvb-si/"
    };
    let path = format!("{base}{subdir}{name}");
    let mut f = File::open(&path).unwrap_or_else(|e| panic!("cannot open {path}: {e}"));
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

    // Exit criterion: EmptyBufferError/DataDelayError stay at zero — those
    // depend only on TBsys draining regularly, which it does throughout.
    // `BufferError` is a documented exception (see the module doc's
    // m6-single.ts section): `tsanalyze` shows this fixture carries no PCR
    // at all, so this test's 40 µs/packet timing is invented, not captured,
    // and TBsys occupancy cannot be judged on it — under that invented
    // clock, PID 0x0000 (PAT) repeats once every ~1.6 ms and TBsys (512
    // bytes, 125 000 bytes/s leak) genuinely cannot keep up. This is a
    // property of the invented timing applied to a PCR-less capture, not of
    // the T-STD buffer model itself (see
    // `buffer_error_absent_when_large_section_paced_realistically` in
    // `src/tests.rs` for the same model proving no false positive at
    // realistic pacing).
    let other_tstd_errors: Vec<_> = events
        .iter()
        .filter(|e| {
            e.indicator == Indicator::EmptyBufferError || e.indicator == Indicator::DataDelayError
        })
        .collect();
    if !other_tstd_errors.is_empty() {
        for e in &other_tstd_errors {
            eprintln!(
                "T-STD event on m6-single.ts: {:?} pid={:?} detail={}",
                e.indicator, e.pid, e.detail
            );
        }
        panic!(
            "m6-single.ts raised {} EmptyBufferError/DataDelayError event(s) on a clean fixture — investigate",
            other_tstd_errors.len()
        );
    }
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

// ── #736: france-tnt-uhf32.ts (real DVB-T broadcast) ─────────────────────

fn read_france_tnt() -> Option<Vec<u8>> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    // Navigate: dvb-conformance/ → workspace root → .test-streams/
    let path = manifest_dir
        .parent() // dvb-conformance's parent = workspace root
        .unwrap()
        .join(".test-streams")
        .join("france-tnt-uhf32.ts");
    let mut f = File::open(&path).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// THE HARD GATE: a conformant real DVB-T broadcast produces ZERO new-indicator
/// events for #736 (EitPfError, NitOtherError, SdtOtherError, EitOtherError,
/// SiMinGapError). A real broadcast meets these requirements; if any fire,
/// the check is wrong, not the stream.
#[test]
fn france_tnt_uhf32_clean_on_new_736_indicators() {
    let data = match read_france_tnt() {
        Some(d) => d,
        None => {
            eprintln!(
                "SKIP: .test-streams/france-tnt-uhf32.ts not found — fetch the private test-streams fixture",
            );
            return;
        }
    };

    let mut monitor = ConformanceMonitor::new();
    let n_packets = data.len() / TS_PACKET_SIZE;

    let new_indicators: &[Indicator] = &[
        Indicator::EitPfError,
        Indicator::NitOtherError,
        Indicator::SdtOtherError,
        Indicator::EitOtherError,
        Indicator::SiMinGapError,
    ];

    let mut violations: Vec<dvb_conformance::ConformanceEvent> = Vec::new();

    for i in 0..n_packets {
        let start = i * TS_PACKET_SIZE;
        let end = start + TS_PACKET_SIZE;
        if end > data.len() {
            break;
        }
        let t = Duration::from_micros(i as u64 * INTER_PACKET_US);
        let events = monitor.feed(&data[start..end], t);
        for e in events {
            if new_indicators.contains(&e.indicator) {
                violations.push(e.clone());
            }
        }
    }

    if !violations.is_empty() {
        for v in &violations {
            eprintln!(
                "#736 violation on france-tnt-uhf32.ts: {:?} pid={:?} at={:?} detail={}",
                v.indicator, v.pid, v.at, v.detail
            );
        }
        panic!(
            "france-tnt-uhf32.ts raised {} #736-indicator violation(s) on a clean real stream — the check is wrong, not the stream",
            violations.len()
        );
    }
}
