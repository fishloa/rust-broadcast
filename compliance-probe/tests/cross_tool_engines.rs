//! Cross-tool equivalence after #2476: this crate's `Probe` and
//! `media_doctor::WatchState` now run the **same** engines — `dvb-conformance`
//! for TR 101 290 accounting and `scte35-splice`'s tracker for cue sanity —
//! and this test pins that they really do give the same answers on the same
//! bytes, rather than each arriving at its own.
//!
//! `fixtures/ts/scte35-real.ts` is the fixture media-doctor's own
//! `Scte35Check` tests use: the canonical industry `splice_insert` vector
//! (`0x4800008f`, an "out" with `break_duration.auto_return = true`) packed
//! into real TS framing on PID `0x01F0`. The one thing it lacks is a PMT, so
//! neither tool's *live* path routes the cue to a tracker; the test instead
//! drives the shared tracker directly on the fixture's own section, and drives
//! both conformance monitors over every packet and every cue, which is the
//! part the two tools genuinely share end to end.
//!
//! # The clock
//!
//! Both monitors are given the identical caller clock (40 µs per packet, a
//! full-DVB-multiplex arrival rate, the same shape
//! `tests/wasm_analyzer_equivalence.rs` and `examples/fixture_report.rs`
//! use), so any difference below is a real divergence in the accounting and
//! not a clock artefact.

use core::time::Duration;
use std::collections::BTreeMap;

use broadcast_common::Parse;
use compliance_probe::Probe;
use media_doctor::{Scte35Check, Severity, WatchState};
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};

/// `table_id` of a `splice_info_section` (ANSI/SCTE 35 §9.6.1).
const SPLICE_INFO_SECTION_TABLE_ID: u8 = 0xFC;

/// Arrival interval of one 188-byte packet: 40 µs is ~37.6 Mbit/s, a full DVB
/// multiplex rate.
const PACKET_INTERVAL: Duration = Duration::from_micros(40);

/// Bytes before a section's payload: `table_id`, then the two bytes carrying
/// the flags and the 12-bit `section_length` (ISO/IEC 13818-1 §2.4.4.1).
const SECTION_HEADER_LEN: usize = 3;

/// Mask for the 4 most-significant `section_length` bits (ISO/IEC 13818-1
/// §2.4.4.1).
const SECTION_LENGTH_HI_MASK: usize = 0x0F;

/// The canonical vector's own `splice_event_id` (see media-doctor's
/// `tests/it/scte35_fixture.rs`).
const CANONICAL_EVENT_ID: u32 = 0x4800_008f;

/// A reference "now" a full hour ahead of the vector's target, on the
/// fixture's own 90 kHz clock.
const NOW_ONE_HOUR_LATER: u64 = 90_000 * 3_600;

fn fixture() -> Vec<u8> {
    test_fixtures::read("ts/scte35-real.ts")
}

fn clock_of(index: usize) -> Duration {
    PACKET_INTERVAL * u32::try_from(index).expect("packet index fits u32")
}

fn watchdog(bytes: &[u8]) -> WatchState {
    let mut state = WatchState::new();
    for (i, chunk) in bytes.chunks(TS_PACKET_SIZE).enumerate() {
        state.feed_datagram(chunk, clock_of(i));
    }
    state
}

fn probe(bytes: &[u8]) -> Probe {
    let mut probe = Probe::new();
    for (i, chunk) in bytes.chunks(TS_PACKET_SIZE).enumerate() {
        let Ok(packet) = <&[u8; TS_PACKET_SIZE]>::try_from(chunk) else {
            break;
        };
        probe.feed_ts_packet(packet, clock_of(i));
    }
    probe
}

/// The fixture's `splice_info_section` bytes, taken off the packet that
/// carries them.
fn fixture_section(bytes: &[u8]) -> Vec<u8> {
    for chunk in bytes.chunks(TS_PACKET_SIZE) {
        let Ok(ts_packet) = TsPacket::parse(chunk) else {
            continue;
        };
        let Some(payload) = ts_packet.payload else {
            continue;
        };
        let Some(start) = payload
            .iter()
            .position(|&b| b == SPLICE_INFO_SECTION_TABLE_ID)
        else {
            continue;
        };
        let Some(length_bytes) = payload.get(start + 1..start + 3) else {
            continue;
        };
        let section_length = ((usize::from(length_bytes[0]) & SECTION_LENGTH_HI_MASK) << 8)
            | usize::from(length_bytes[1]);
        if let Some(section) = payload.get(start..start + SECTION_HEADER_LEN + section_length) {
            return section.to_vec();
        }
    }
    panic!("fixture carries no splice_info_section");
}

/// A named per-indicator count map, so two differently-shaped collections can
/// be compared directly.
/// The same fixture, one caller clock: `media_doctor::WatchState` and
/// `compliance_probe::Probe` must raise the same TR 101 290 events under the
/// same indicator names.
#[test]
fn both_tools_raise_the_same_indicator_counts_on_one_fixture() {
    let bytes = fixture();
    let state = watchdog(&bytes);
    let probe = probe(&bytes);

    let snapshot = state.snapshot();
    let watch: BTreeMap<&'static str, u64> = snapshot
        .conformance
        .iter()
        .map(|c| (c.indicator, c.count))
        .collect();
    let probe_counts: BTreeMap<&'static str, u64> = probe
        .indicator_counts()
        .iter()
        .map(|c| (c.indicator, c.count))
        .collect();
    assert_eq!(watch, probe_counts, "per-indicator counts must agree");

    let stats = probe.conformance_stats();
    assert_eq!(
        stats.packets,
        u64::try_from(bytes.len() / TS_PACKET_SIZE).expect("fits")
    );
    assert_eq!(
        stats.events,
        probe_counts.values().sum::<u64>(),
        "the probe's own event total is exactly its per-indicator sum"
    );
    assert_eq!(
        stats.in_sync, snapshot.in_sync,
        "both read the same TR 101 290 §5.2 indicator 1.1 hysteresis state"
    );
}

/// The same cue, one shared tracker: the splice-balance findings
/// media-doctor's `Scte35Check` reports are exactly what the tracker
/// `compliance-probe` reads produces.
#[test]
fn both_tools_agree_on_the_canonical_cues_splice_state() {
    let bytes = fixture();
    let section = fixture_section(&bytes);

    /// `pointer_field` before the first section in a payload (ISO/IEC
    /// 13818-1 §2.4.4.2).
    const POINTER_FIELD_LEN: usize = 1;

    let mut tracker = scte35_splice::SpliceTracker::default();
    let mut observations = Vec::new();
    let mut payload = vec![0u8; POINTER_FIELD_LEN];
    payload.extend_from_slice(&section);
    tracker.feed(&payload, true, |observation| observations.push(observation));
    assert_eq!(observations.len(), 1, "the fixture carries one cue");
    assert_eq!(observations[0].event_id, CANONICAL_EVENT_ID);
    assert!(!observations[0].duplicate_open);
    assert_eq!(
        tracker.open_count(),
        0,
        "the vector's auto-returning out closes at its own duration"
    );

    let mut report = media_doctor::Report::new();
    media_doctor::Diagnostic::run(&Scte35Check, &bytes, &mut report);
    assert!(
        report
            .findings()
            .iter()
            .all(|f| f.rule_id != "scte35-unbalanced" && f.rule_id != "scte35-dup-out"),
        "no splice-balance finding for a self-closing break; got {:?}",
        report.findings()
    );
    assert!(
        report
            .findings()
            .iter()
            .all(|f| f.severity != Severity::Error),
        "a complete, self-closing break is a clean stream"
    );
    let cue_pid = Pid::from_const(0x01F0);
    assert!(
        bytes
            .chunks(TS_PACKET_SIZE)
            .any(|chunk| TsPacket::parse(chunk).is_ok_and(|p| p.header.pid == cue_pid)),
        "the cue is carried on the conventional PID both tools look at here"
    );
}

/// Cue timing: the shared engine's verdict for the fixture's cue is the same
/// judgement whichever tool asks. The vector's target is a stable time far in
/// the fixture's past, so a reference "now" an hour earlier reads it as still
/// ahead and one an hour later reads it as late.
#[test]
fn both_tools_agree_on_where_the_cue_lands_on_the_clock() {
    let section = fixture_section(&fixture());
    let sis = scte35_splice::SpliceInfoSection::parse(&section).expect("parse the cue");
    let event = scte35_splice::splice_event(
        &sis.clear.as_ref().expect("clear section").command,
        sis.pts_adjustment,
    )
    .expect("a splice_insert");
    let target = event.times.first().copied().expect("a timed cue");

    assert!(scte35_splice::judge_splice_time(target, target));
    assert!(
        scte35_splice::judge_splice_time(target, target.saturating_sub(NOW_ONE_HOUR_LATER)),
        "an hour before its target the cue is still ahead"
    );
    assert!(
        !scte35_splice::judge_splice_time(target, target + NOW_ONE_HOUR_LATER),
        "an hour past its target the same cue is late"
    );
}
