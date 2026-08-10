//! Cross-tool equivalence: this crate's `Probe` vs. the `demo/` WASM analyzer,
//! over the same committed real capture (`fixtures/ts/m6-single.ts`).
//!
//! # Why this test exists
//!
//! Two independent tools in this repository run ETSI TR 101 290 over the same
//! fixture and report **different total event counts** — 876 here, 911 there.
//! That is exactly the shape of an under-reporting defect in a compliance
//! probe (a probe that silently misses 35 events is indistinguishable from a
//! clean stream), so the difference is pinned here as an executable fact
//! rather than left as prose anyone can rationalise.
//!
//! # The mechanism, measured — not the config
//!
//! Both tools construct `dvb_conformance::ConformanceMonitor::new()`, i.e.
//! the **identical default [`dvb_conformance::Config`]**: same PAT/PMT
//! intervals, same PCR repetition/discontinuity limits, same SI repetition
//! intervals. There is no threshold difference to explain anything, and this
//! test would be worthless if there were — it would just be pinning two
//! arbitrary knob settings against each other.
//!
//! The entire difference is the **caller-supplied clock**, which
//! `ConformanceMonitor::feed(packet, t)` takes and which every timeout-based
//! indicator is evaluated against:
//!
//! - `demo/src/lib.rs::analyze_impl` anchors its clock on **observed PCR
//!   values** (27 MHz → seconds), falling back to `+1 nanosecond per packet`
//!   until the first PCR is seen.
//! - **`m6-single.ts` contains no PCR at all** (asserted below: 95 packets
//!   carry an adaptation field, 0 carry a PCR). So that fallback is never
//!   escaped, and the analyzer models the whole 1264-packet capture as
//!   spanning **1.264 µs** — an implied ~1.5 Tbit/s.
//!
//! At that implied rate the T-STD system transport buffer TBsys (512 bytes,
//! draining at 1 Mbit/s per ISO/IEC 13818-1 §2.4.2.4) cannot possibly keep
//! up, so it overflows — and *every one* of the 35 extra events is
//! `Buffer_error` (TR 101 290 Table 5.0c indicator 3.3). Nothing else
//! differs.
//!
//! # The invariant worth having
//!
//! `Continuity_count_error` — the structural finding about this stream — is
//! **876 at every clock rate tested**, from a frozen clock to 2 ms/packet.
//! It is clock-independent, and the two tools agree on it exactly. What is
//! clock-dependent is precisely the T-STD buffer-model indicator, which is
//! a statement about *arrival timing* and therefore cannot be answered
//! without an honest arrival clock. Both readings are correct answers to
//! different questions; this test pins both so neither tool can drift from
//! the other unnoticed.

use core::time::Duration;

use dvb_conformance::ConformanceMonitor;
use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};
use std::collections::BTreeMap;

fn fixture() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts");
    std::fs::read(path).unwrap_or_else(|e| panic!("committed fixture {path}: {e}"))
}

/// Replicates `demo/src/lib.rs::analyze_impl`'s clock **exactly**: anchored on
/// observed PCR values, `+1 ns` per packet until the first PCR appears.
///
/// Kept as a verbatim transcription of that function's clock arithmetic (not
/// a paraphrase) so that if the demo's clock model ever changes, this test's
/// pinned numbers stop matching it and someone has to reconcile the two.
fn run_wasm_analyzer_clock(bytes: &[u8]) -> BTreeMap<&'static str, u64> {
    let mut conformance = ConformanceMonitor::new();
    let mut acc: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut clock = Duration::ZERO;
    let mut pcr_anchor: Option<(u64, Duration)> = None;

    for chunk in bytes.chunks(TS_PACKET_SIZE) {
        if chunk.len() < TS_PACKET_SIZE {
            break;
        }
        let Ok(ts_packet) = TsPacket::parse(chunk) else {
            continue;
        };
        if let Some(Ok(af)) = ts_packet.adaptation_field()
            && let Some(pcr) = af.pcr
        {
            let pcr_27mhz = pcr.as_27mhz();
            if let Some((anchor_val, anchor_t)) = pcr_anchor
                && pcr_27mhz >= anchor_val
            {
                let delta_secs = (pcr_27mhz - anchor_val) as f64 / 27_000_000.0;
                let candidate = anchor_t + Duration::from_secs_f64(delta_secs);
                if candidate > clock {
                    clock = candidate;
                }
            }
            pcr_anchor = Some((pcr_27mhz, clock));
        }
        if pcr_anchor.is_none() {
            clock += Duration::from_nanos(1);
        }
        for ev in conformance.feed(chunk, clock) {
            *acc.entry(ev.indicator.name()).or_insert(0) += 1;
        }
    }
    acc
}

/// A fixed arrival rate, in nanoseconds per 188-byte packet — the clock shape
/// a live probe actually has (wall-clock arrival), and what
/// `examples/fixture_report.rs` uses.
fn run_fixed_rate(bytes: &[u8], nanos_per_packet: u64) -> BTreeMap<&'static str, u64> {
    let mut conformance = ConformanceMonitor::new();
    let mut acc: BTreeMap<&'static str, u64> = BTreeMap::new();
    for (i, chunk) in bytes.chunks(TS_PACKET_SIZE).enumerate() {
        if chunk.len() < TS_PACKET_SIZE {
            break;
        }
        let t = Duration::from_nanos(i as u64 * nanos_per_packet);
        for ev in conformance.feed(chunk, t) {
            *acc.entry(ev.indicator.name()).or_insert(0) += 1;
        }
    }
    acc
}

fn total(acc: &BTreeMap<&'static str, u64>) -> u64 {
    acc.values().sum()
}

/// The fixture must carry **no PCR** — this is the load-bearing premise of
/// the whole explanation. If a future fixture change introduced PCRs, the
/// demo's clock would stop degenerating and every pinned number below would
/// become meaningless, so this is asserted first and explicitly rather than
/// assumed.
#[test]
fn fixture_carries_no_pcr_so_the_analyzer_clock_degenerates() {
    let data = fixture();
    assert_eq!(data.len() % TS_PACKET_SIZE, 0, "fixture is whole packets");
    assert_eq!(data.len() / TS_PACKET_SIZE, 1264, "fixture packet count");

    let mut with_af = 0u32;
    let mut with_pcr = 0u32;
    for chunk in data.chunks(TS_PACKET_SIZE) {
        let p = TsPacket::parse(chunk).expect("every packet in this fixture parses");
        if p.header.has_adaptation {
            with_af += 1;
        }
        if let Some(Ok(af)) = p.adaptation_field()
            && af.pcr.is_some()
        {
            with_pcr += 1;
        }
    }
    assert_eq!(with_af, 95, "packets carrying an adaptation field");
    assert_eq!(
        with_pcr, 0,
        "fixture must carry no PCR — the demo analyzer's PCR-anchored clock \
         degenerating to its 1 ns/packet fallback is the entire reason its \
         total differs from this crate's"
    );
}

/// Reproduce the WASM analyzer's reading **exactly**: 911 total, and the
/// per-indicator split showing where every one of those events comes from.
#[test]
fn reproduces_the_wasm_analyzer_reading_exactly() {
    let acc = run_wasm_analyzer_clock(&fixture());

    assert_eq!(
        total(&acc),
        911,
        "must reproduce the demo WASM analyzer's total exactly; got {acc:?}"
    );
    assert_eq!(acc.get("Continuity_count_error").copied(), Some(876));
    assert_eq!(acc.get("Buffer_error").copied(), Some(35));
    assert_eq!(
        acc.len(),
        2,
        "exactly two indicators fire under the analyzer's clock; got {acc:?}"
    );
}

/// Under a *physically plausible* arrival clock, the reading is 876 — and the
/// entire 35-event difference is `Buffer_error`, nothing else. This is the
/// assertion that would fail if this crate were genuinely under-reporting
/// some other indicator.
#[test]
fn under_a_real_arrival_clock_the_only_difference_is_the_tstd_buffer_indicator() {
    let data = fixture();
    // 40 µs/packet ≈ 37.6 Mbit/s — a full-multiplex rate, and what
    // `examples/fixture_report.rs` uses.
    let real = run_fixed_rate(&data, 40_000);
    let analyzer = run_wasm_analyzer_clock(&data);

    assert_eq!(total(&real), 876);
    assert_eq!(real.get("Continuity_count_error").copied(), Some(876));
    assert_eq!(
        real.get("Buffer_error").copied(),
        None,
        "TBsys has time to drain at a real bitrate, so 3.3 must not fire"
    );

    // The difference is exactly and only Buffer_error.
    let mut diff: Vec<(&str, i64)> = Vec::new();
    for (k, v) in &analyzer {
        let other = real.get(k).copied().unwrap_or(0);
        if *v != other {
            diff.push((k, *v as i64 - other as i64));
        }
    }
    assert_eq!(
        diff,
        vec![("Buffer_error", 35)],
        "the whole cross-tool gap must be the T-STD buffer indicator and \
         nothing else — any other entry here is a real indicator-logic \
         divergence, not a clock artefact"
    );
}

/// `Continuity_count_error` is clock-**independent**: the structural finding
/// about this stream is 876 whether the clock is frozen or running at
/// 2 ms/packet. This is the invariant that makes the two tools genuinely
/// mutually checkable — it cannot be tuned away by choosing a clock.
#[test]
fn continuity_count_is_identical_at_every_clock_rate() {
    let data = fixture();
    for &ns in &[
        0u64, 1, 1_000, 10_000, 40_000, 100_000, 1_000_000, 2_000_000,
    ] {
        let acc = run_fixed_rate(&data, ns);
        assert_eq!(
            acc.get("Continuity_count_error").copied(),
            Some(876),
            "Continuity_count_error must be 876 regardless of clock rate \
             (failed at {ns} ns/packet): {acc:?}"
        );
    }
}

/// The T-STD `Buffer_error` count is a monotone function of the assumed
/// arrival rate, decaying to zero once TBsys can drain — documenting that
/// indicator 3.3 answers a question about *arrival timing*, so it is only as
/// meaningful as the clock it is given.
#[test]
fn tstd_buffer_error_decays_as_the_assumed_arrival_rate_becomes_realistic() {
    let data = fixture();
    let frozen = run_fixed_rate(&data, 0);
    let slow = run_fixed_rate(&data, 10_000);
    let realistic = run_fixed_rate(&data, 100_000);

    assert_eq!(frozen.get("Buffer_error").copied(), Some(35));
    assert_eq!(slow.get("Buffer_error").copied(), Some(29));
    assert_eq!(realistic.get("Buffer_error").copied(), None);
}
