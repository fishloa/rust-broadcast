//! What `InputDegradation::ContinuityGap` means, per PID (ISO/IEC 13818-1
//! §2.4.3.3, ETSI TR 101 290 §5.2.1 check 1.4).
//!
//! The rules the streaming core implements, on payload-bearing non-null packets
//! only, per PID:
//!
//! * the counter advances by one per payload packet (`last + 1 mod 16`); a
//!   packet without payload (AFC `00`/`10`) neither advances nor is checked;
//! * ONE byte-identical repeat (PCR field excepted) with the same counter is a
//!   legal duplicate: no event, payload discarded;
//! * every further identical repeat is a fault, **one event per excess
//!   packet** (TR 101 290: "duplicate more than twice"). Before the fix the
//!   run state was reset by the first fault, so a run of N identical repeats
//!   reported only about N/2 events (alternating legal/illegal);
//! * a signalled `discontinuity_indicator` lets a jump through silently;
//! * the null PID `0x1FFF` is exempt;
//! * a same-counter packet that is NOT identical is a fault (`got == last`).
//!
//! The capture expectations were computed ONCE by an independent
//! pure-python walk over the raw bytes (not a dependency of this crate): per
//! PID, first packet seeds the state; counter == last with identical packet
//! bytes is a duplicate (first = legal, later = "repeat"); counter == last with
//! different bytes or counter != last+1 is a gap (unless the packet carries
//! `discontinuity_indicator`); no-payload and null packets skipped. The
//! expected count per PID is `gap + same-counter-different + repeated
//! duplicates`.
//!
//! * `ts/orf3-dvbs2-raw-zenith-tv.ts` (private): PID 0x1C90 (DSM-CC carousel)
//!   only: 118 first duplicates (legal, 0 events), 180 repeated duplicates, 7
//!   real gaps => 187 events, all on PID 0x1C90; PID 0x0C08 has 959 packets
//!   without payload and reports nothing; every other PID is clean.
//! * `ts/m6-single.ts` 879, `ts/france-tnt-dvbt-20s.ts` 139 (private),
//!   `ts/france-tnt-m6-eac3-zenith.ts` 2716 (private),
//!   `ts/hotbird-mhp-si.ts` 21 608 (private; PID 0x0690 carries the PCR - three PCR-only-different repeats are legal duplicates, the script masks the 6 PCR bytes like TS H.222.0 §2.4.3.3): see each
//!   test for the per-PID table.

use std::collections::BTreeMap;

use broadcast_common::Unpackage;
use mpeg_ts::pid::Pid;
use transmux::{DemuxEvent, InputDegradation, StreamingTsDemux, TsDemux};

const TS: usize = 188;
/// Trailing null packets that let the resynchroniser lock (see [`gaps`]).
const PAD: usize = 8;

/// A 188-byte packet: `afc` is the 2-bit adaptation_field_control; with a
/// payload-less `afc == 2` the adaptation field fills the packet.
fn pkt(pid: u16, afc: u8, cc: u8, disc: bool, fill: u8) -> [u8; TS] {
    let mut p = [fill; TS];
    p[0] = 0x47;
    p[1] = ((pid >> 8) as u8) & 0x1F;
    p[2] = pid as u8;
    p[3] = (afc << 4) | (cc & 0x0F);
    match afc {
        2 => {
            p[4] = 183; // adaptation field fills the packet
            p[5] = if disc { 0x80 } else { 0 };
            for b in &mut p[6..] {
                *b = 0xFF;
            }
        }
        3 => {
            p[4] = 1; // flags byte only
            p[5] = if disc { 0x80 } else { 0 };
        }
        _ => {}
    }
    p
}

fn pay(pid: u16, cc: u8) -> [u8; TS] {
    pkt(pid, 1, cc, false, 0xAB)
}

/// `(pid, packet_index, expected, got)` for every continuity gap raised.
fn gaps(packets: &[[u8; TS]]) -> Vec<(u16, u64, u8, u8)> {
    let mut d = StreamingTsDemux::new();
    for p in packets {
        d.feed(p);
    }
    // The resynchroniser needs a run of sync bytes after a packet before it
    // trusts it; trailing null packets (exempt from the check) give short
    // hand-built sequences that run without changing any packet index.
    for _ in 0..PAD {
        d.feed(&pay(0x1FFF, 0));
    }
    d.finish();
    let mut out = Vec::new();
    while let Some(e) = d.poll_event() {
        if let DemuxEvent::InputDegraded {
            kind: InputDegradation::ContinuityGap { expected, got },
            provenance,
            ..
        } = e
        {
            out.push((
                provenance.pid.expect("TS provenance carries the PID").value(),
                provenance.packet_index.expect("packet index"),
                expected,
                got,
            ));
        }
    }
    out
}

#[test]
fn counting_up_is_clean_and_wraps() {
    let v: Vec<_> = (0..40).map(|i| pay(0x100, i)).collect();
    assert!(gaps(&v).is_empty());
}

#[test]
fn one_duplicate_is_legal() {
    let v = [pay(0x100, 0), pay(0x100, 1), pay(0x100, 1), pay(0x100, 2)];
    assert!(gaps(&v).is_empty());
}

#[test]
fn every_repeat_beyond_the_one_duplicate_is_a_fault() {
    // original + 5 identical repeats: repeat 1 is the legal duplicate, repeats
    // 2..=5 are four faults - one per packet, NOT alternating.
    let mut v = vec![pay(0x100, 0), pay(0x100, 1)];
    v.extend(std::iter::repeat_n(pay(0x100, 1), 5));
    v.push(pay(0x100, 2));
    assert_eq!(
        gaps(&v),
        vec![
            (0x100, 3, 2, 1),
            (0x100, 4, 2, 1),
            (0x100, 5, 2, 1),
            (0x100, 6, 2, 1)
        ]
    );
}

#[test]
fn run_state_resets_on_a_fresh_original() {
    // a new original re-arms the one legal duplicate.
    let v = [
        pay(0x100, 0),
        pay(0x100, 0),
        pay(0x100, 0), // fault
        pay(0x100, 1),
        pay(0x100, 1), // legal again
    ];
    assert_eq!(gaps(&v), vec![(0x100, 2, 1, 0)]);
}

#[test]
fn real_gap_reports_expected_and_got() {
    let v = [pay(0x100, 3), pay(0x100, 4), pay(0x100, 7), pay(0x100, 8)];
    assert_eq!(gaps(&v), vec![(0x100, 2, 5, 7)]);
}

#[test]
fn same_counter_different_bytes_is_a_fault() {
    let v = [
        pay(0x100, 3),
        pkt(0x100, 1, 3, false, 0xCD), // same CC, different payload
    ];
    assert_eq!(gaps(&v), vec![(0x100, 1, 4, 3)]);
}

#[test]
fn packets_without_payload_do_not_advance_or_get_checked() {
    let v = [
        pay(0x100, 0),
        pkt(0x100, 2, 0, false, 0), // AFC 10: CC stays 0, no payload
        pkt(0x100, 2, 0, false, 0), // repeated, byte-identical: not a "duplicate"
        pkt(0x100, 2, 9, false, 0), // junk counter: ignored without payload
        pay(0x100, 1),
    ];
    assert!(gaps(&v).is_empty());
}

#[test]
fn discontinuity_indicator_lets_a_jump_through() {
    let v = [
        pay(0x100, 0),
        pkt(0x100, 3, 9, true, 0x11), // signalled jump 0 -> 9
        pay(0x100, 10),               // continues from the new baseline
        pay(0x100, 12),               // unsignalled jump: fault
    ];
    assert_eq!(gaps(&v), vec![(0x100, 3, 11, 12)]);
}

#[test]
fn null_packets_are_exempt() {
    let v = [pay(0x1FFF, 0), pay(0x1FFF, 9), pay(0x1FFF, 9), pay(0x1FFF, 9)];
    assert!(gaps(&v).is_empty());
}

#[test]
fn pids_are_tracked_independently_and_attributed() {
    let v = [
        pay(0x100, 0),
        pay(0x200, 5),
        pay(0x100, 1),
        pay(0x200, 9), // fault on 0x200 only
        pay(0x100, 2),
    ];
    assert_eq!(gaps(&v), vec![(0x200, 3, 6, 9)]);
}

#[test]
fn media_degraded_at_carries_the_pid_parallel_to_degraded() {
    let mut bytes = Vec::new();
    for p in [pay(0x100, 0), pay(0x100, 4), pay(0x200, 0), pay(0x200, 1)] {
        bytes.extend_from_slice(&p);
    }
    for _ in 0..PAD {
        bytes.extend_from_slice(&pay(0x1FFF, 0));
    }
    let media = TsDemux::new().unpackage(&bytes).unwrap();
    assert_eq!(media.degraded.len(), 1);
    assert_eq!(media.degraded_at.len(), media.degraded.len());
    assert_eq!(media.degraded_at[0].kind, media.degraded[0]);
    assert_eq!(media.degraded_at[0].provenance.pid, Some(Pid::new(0x100)));
    assert_eq!(media.degraded_at[0].provenance.packet_index, Some(1));
}

// ── real captures ──────────────────────────────────────────────────────────

fn per_pid(media: &transmux::Media) -> BTreeMap<u16, usize> {
    let mut m = BTreeMap::new();
    for r in &media.degraded_at {
        if let InputDegradation::ContinuityGap { .. } = r.kind {
            *m.entry(r.provenance.pid.unwrap().value()).or_insert(0) += 1;
        }
    }
    m
}

fn expect(table: &[(u16, usize)]) -> BTreeMap<u16, usize> {
    table.iter().copied().collect()
}

#[test]
fn orf3_carousel_damage_is_all_on_pid_0x1c90() {
    let Some(path) = test_fixtures::require(
        "ts/orf3-dvbs2-raw-zenith-tv.ts",
        "ts_continuity_semantics orf3",
    ) else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let media = TsDemux::new().unpackage(&bytes).unwrap();
    assert_eq!(per_pid(&media), expect(&[(0x1C90, 187)]));
    // 180 excess repeats (got == last) + 7 real gaps
    let repeats = media
        .degraded
        .iter()
        .filter(|k| {
            matches!(k, InputDegradation::ContinuityGap { expected, got }
                if (got.wrapping_add(1) & 0x0F) == *expected)
        })
        .count();
    assert_eq!(repeats, 180);
}

fn check_capture(rel: &str, total: usize, table: &[(u16, usize)]) {
    let Some(path) = test_fixtures::require(rel, "ts_continuity_semantics") else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let media = TsDemux::new().unpackage(&bytes).unwrap();
    assert_eq!(per_pid(&media), expect(table), "{rel}");
    assert_eq!(per_pid(&media).values().sum::<usize>(), total, "{rel}");
}

#[test]
fn m6_single_matches_the_independent_count() {
    check_capture(
        "ts/m6-single.ts",
        879,
        &[
            (0x0064, 8),
            (0x0082, 321),
            (0x0083, 245),
            (0x0084, 225),
            (0x008C, 70),
            (0x00AA, 3),
            (0x00AB, 6),
            (0x00AC, 1),
        ],
    );
}

#[test]
fn france_tnt_dvbt_20s_matches_the_independent_count() {
    check_capture(
        "ts/france-tnt-dvbt-20s.ts",
        139,
        &[
            (0x0012, 2),
            (0x0078, 8),
            (0x0082, 9),
            (0x0083, 7),
            (0x0084, 8),
            (0x00C8, 2),
            (0x00DC, 10),
            (0x00E6, 5),
            (0x00E7, 5),
            (0x00E8, 5),
            (0x00FA, 1),
            (0x010F, 4),
            (0x01F4, 2),
            (0x0208, 8),
            (0x0212, 7),
            (0x0213, 7),
            (0x0258, 1),
            (0x026C, 10),
            (0x0276, 5),
            (0x0277, 5),
            (0x0278, 6),
            (0x028B, 1),
            (0x02BC, 2),
            (0x02D0, 10),
            (0x02DA, 9),
        ],
    );
}

#[test]
fn hotbird_mhp_si_matches_the_independent_count() {
    check_capture(
        "ts/hotbird-mhp-si.ts",
        21608,
        &[
            (0x0011, 1),
            (0x03AC, 1771),
            (0x03D4, 123),
            (0x03E8, 1924),
            (0x064A, 1835),
            (0x0654, 1836),
            (0x065E, 1810),
            (0x0686, 1744),
            (0x0690, 1837),
            (0x06C2, 1412),
            (0x06E0, 1796),
            (0x06EA, 1790),
            (0x06F4, 1800),
            (0x0712, 1929),
        ],
    );
}

#[test]
fn france_tnt_m6_eac3_zenith_matches_the_independent_count() {
    check_capture(
        "ts/france-tnt-m6-eac3-zenith.ts",
        2716,
        &[
            (0x0064, 8),
            (0x0078, 1837),
            (0x0082, 321),
            (0x0083, 245),
            (0x0084, 225),
            (0x008C, 70),
            (0x00AA, 3),
            (0x00AB, 6),
            (0x00AC, 1),
        ],
    );
}
