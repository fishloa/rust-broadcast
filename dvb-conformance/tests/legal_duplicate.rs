//! Real-fixture proof for issue #956: the ISO/IEC 13818-1:2023 §2.4.3.3
//! "legal duplicate" rule for Continuity_count_error (indicator 1.4).
//!
//! §2.4.3.3: "In duplicate packets each byte of the original packet shall be
//! duplicated, with the exception that in the program clock reference
//! fields, if present, a valid value shall be encoded." A packet repeating
//! the previous CC is legal ONLY when it is byte-identical apart from the
//! PCR field — comparing CC alone (the pre-fix behaviour) accepts ANY
//! same-CC repeat, silently swallowing genuine continuity faults.
//!
//! Two properties are proven here, both against real broadcast bytes:
//!
//! 1. [`m6_duplicate_distinguishes_legal_from_illegal_same_cc_repeats`] —
//!    the committed `m6-duplicate.ts` real capture carries 5 genuinely
//!    byte-identical same-CC repeats (legal duplicates) AND 77 same-CC
//!    repeats whose payload differs (real continuity faults). The fixed
//!    monitor must flag the 77 and only the 77.
//! 2. [`pcr_only_difference_is_still_a_legal_duplicate`] /
//!    [`non_pcr_difference_on_same_cc_is_not_a_legal_duplicate`] — built
//!    from one real PCR-bearing packet lifted from `france2.ts` (no
//!    fixture in this repo's committed captures, nor in the two large
//!    private `.test-streams` real broadcast captures scanned for this fix,
//!    naturally contains a re-encoded-PCR duplicate — see PR discussion).
//!    Byte-for-byte real capture data; only the specific field under test is
//!    altered.

use core::time::Duration;
use std::fs;
use std::path::PathBuf;

use dvb_conformance::{ConformanceMonitor, Indicator};

const TS_PACKET_SIZE: usize = 188;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("ts")
        .join(name)
}

/// A syntactically valid, otherwise-inert TS packet: sync byte + PID
/// 0x1FFE (never used by the fixtures under test) + afc `01` (payload
/// only) + cc 0 + all-zero payload.
fn inert_packet() -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0u8; TS_PACKET_SIZE];
    pkt[0] = 0x47;
    pkt[1] = 0x1F; // PID[12:8] of 0x1FFE, tei/pusi/priority = 0
    pkt[2] = 0xFE; // PID[7:0]
    pkt[3] = 0x10; // scrambling=00, afc='01' (payload only), cc=0
    pkt
}

/// [`ConformanceMonitor::feed`] suppresses every indicator, including
/// `Continuity_count_error`, until the sync-acquisition hysteresis is
/// satisfied (`Config::sync_acquire_packets`, default 5 consecutive
/// correct-sync packets — see §1.1). A 2-packet test therefore sees NO
/// events at all regardless of the CC logic under test, which would
/// silently mask a bug rather than exercise it. Feed a 5-packet inert
/// preamble on an unrelated PID first, exactly as
/// `dvb-conformance/src/tests.rs`'s own `acquire_sync` helper does, so the
/// packets under test are actually evaluated.
fn cc_error_count(packets: &[[u8; TS_PACKET_SIZE]]) -> usize {
    let mut monitor = ConformanceMonitor::new();
    let mut count = 0;
    let preamble = [inert_packet(); 5];
    for (i, pkt) in preamble.iter().chain(packets.iter()).enumerate() {
        let t = Duration::from_micros(i as u64 * 40);
        let events = monitor.feed(pkt, t);
        count += events
            .iter()
            .filter(|e| e.indicator == Indicator::ContinuityCountError)
            .count();
    }
    count
}

// ── 1. Real capture: distinguishes legal dup vs genuine CC fault ───────────

/// Independently re-derives, from the raw fixture bytes (not via
/// `dvb-conformance`), which same-CC repeats on `m6-duplicate.ts` are
/// legal duplicates (byte-identical, PCR excepted) vs genuine continuity
/// faults (payload differs). This mirrors the reference oracle already used
/// by `ts-fix/tests/cc_repair.rs` (`hash_payload_skip_pcr`), confirming the
/// fixture's known "5 legal duplicates" property independently of the code
/// under test, then additionally counts the illegal same-CC repeats the old
/// `dvb-conformance` code silently missed.
fn classify_same_cc_repeats(data: &[u8]) -> (usize, usize) {
    fn has_payload(pkt: &[u8]) -> bool {
        (pkt[3] & 0x10) != 0
    }
    fn has_adaptation(pkt: &[u8]) -> bool {
        (pkt[3] & 0x20) != 0
    }
    fn canonical(pkt: &[u8]) -> Vec<u8> {
        let mut buf = pkt.to_vec();
        if has_adaptation(pkt) && pkt[4] > 0 {
            let flags = pkt[5];
            let has_pcr = (flags & 0x10) != 0;
            if has_pcr {
                for b in &mut buf[6..12] {
                    *b = 0;
                }
            }
        }
        buf
    }

    let mut last: std::collections::BTreeMap<u16, (u8, Vec<u8>)> =
        std::collections::BTreeMap::new();
    let mut legal = 0usize;
    let mut illegal = 0usize;
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() < TS_PACKET_SIZE || !has_payload(chunk) {
            continue;
        }
        let pid = (((chunk[1] & 0x1F) as u16) << 8) | chunk[2] as u16;
        let cc = chunk[3] & 0x0F;
        let canon = canonical(chunk);
        if let Some((last_cc, last_canon)) = last.get(&pid)
            && cc == *last_cc
        {
            if canon == *last_canon {
                legal += 1;
            } else {
                illegal += 1;
            }
        }
        last.insert(pid, (cc, canon));
    }
    (legal, illegal)
}

#[test]
fn m6_duplicate_distinguishes_legal_from_illegal_same_cc_repeats() {
    let data = fs::read(fixture_path("m6-duplicate.ts")).expect("m6-duplicate.ts not found");

    // Oracle: independently re-derive legal vs illegal same-CC repeats
    // straight from the raw bytes.
    let (legal, illegal) = classify_same_cc_repeats(&data);
    assert_eq!(
        legal, 5,
        "m6-duplicate.ts is known (ts-fix/tests/cc_repair.rs) to carry 5 legal duplicates"
    );
    assert!(
        illegal > 0,
        "m6-duplicate.ts must also carry real same-CC-but-different-payload repeats \
         for this test to prove anything"
    );

    // The code under test: feed the real capture through ConformanceMonitor
    // and count Continuity_count_error (1.4) events.
    let packets: Vec<[u8; TS_PACKET_SIZE]> = data
        .chunks(TS_PACKET_SIZE)
        .filter(|c| c.len() == TS_PACKET_SIZE)
        .map(|c| c.try_into().unwrap())
        .collect();
    let cc_errors = cc_error_count(&packets);

    // Bug #956: the pre-fix `is_duplicate = cc == last_cc && has_payload`
    // never compared payload bytes, so ALL 82 same-CC repeats (5 legal + 77
    // illegal) were silently accepted and this count was 0. Fixed, it must
    // report at least the `illegal` count (it may exceed it slightly: the
    // "second consecutive duplicate" rule (§2.4.3.3: at most 2 consecutive
    // same-CC packets) can also fire independently on this fixture).
    assert!(
        cc_errors >= illegal,
        "expected at least {illegal} Continuity_count_error events on the real \
         same-CC-but-different-payload repeats in m6-duplicate.ts, got {cc_errors} \
         — the legal-duplicate byte comparison regressed"
    );
    assert!(
        cc_errors > 0,
        "m6-duplicate.ts must raise Continuity_count_error under the fixed rule \
         (it raised zero under the pre-#956-fix rule)"
    );
}

// ── 2. Real packet, PCR-only vs genuine-payload difference (bite test) ──

/// One real PCR-bearing packet lifted verbatim from `fixtures/ts/france2.ts`
/// packet index 0 (PID 0x0078, afc `11`, `adaptation_field_length` 7, PCR
/// present at the canonical offset). Bytes are exactly as captured.
fn real_pcr_packet() -> [u8; TS_PACKET_SIZE] {
    let data = fs::read(fixture_path("france2.ts")).expect("france2.ts not found");
    let pkt: [u8; TS_PACKET_SIZE] = data[0..TS_PACKET_SIZE].try_into().unwrap();
    // Sanity: this is the packet the test was designed around. If the
    // fixture ever changes, fail loudly here rather than silently testing
    // the wrong bytes.
    assert_eq!(pkt[3] & 0x0F, 0x0F, "expected cc=15 on france2.ts packet 0");
    assert_eq!(
        pkt[3] & 0x30,
        0x30,
        "expected adaptation_field_control='11' on france2.ts packet 0"
    );
    assert!(pkt[4] > 0, "expected a non-empty adaptation field");
    assert_ne!(
        pkt[5] & 0x10,
        0,
        "expected PCR_flag set on france2.ts packet 0"
    );
    pkt
}

#[test]
fn pcr_only_difference_is_still_a_legal_duplicate() {
    let original = real_pcr_packet();
    let mut duplicate = original;
    // Re-encode the PCR field only (bytes 6..12: program_clock_reference_base
    // + reserved + program_clock_reference_extension — ISO/IEC 13818-1:2023
    // §2.4.3.5). Same CC, same everything else.
    for b in &mut duplicate[6..12] {
        *b ^= 0xFF;
    }
    assert_ne!(
        original[6..12],
        duplicate[6..12],
        "sanity: PCR bytes must actually differ"
    );
    assert_eq!(
        original[TS_PACKET_SIZE.min(12)..],
        duplicate[TS_PACKET_SIZE.min(12)..],
        "sanity: everything after the PCR field must still be byte-identical"
    );

    let cc_errors = cc_error_count(&[original, duplicate]);
    assert_eq!(
        cc_errors, 0,
        "a duplicate that differs ONLY in its re-encoded PCR field is legal \
         per §2.4.3.3 and must NOT raise Continuity_count_error"
    );
}

#[test]
fn non_pcr_difference_on_same_cc_is_not_a_legal_duplicate() {
    let original = real_pcr_packet();

    // Baseline: PCR-only difference is accepted (no error) — proven above,
    // re-confirmed here as the "before mutation" pass state for the bite
    // test below.
    let mut duplicate = original;
    for b in &mut duplicate[6..12] {
        *b ^= 0xFF;
    }
    assert_eq!(
        cc_error_count(&[original, duplicate]),
        0,
        "baseline: PCR-only difference must be accepted before mutating the payload"
    );

    // Mutate: flip a payload byte (well past the adaptation field, which
    // ends at offset 5 + af_len = 5 + 7 = 12) while leaving CC and the PCR
    // field alone. This is no longer byte-identical outside the PCR
    // exemption, so it must now raise Continuity_count_error.
    let mutate_offset = 20;
    assert!(
        mutate_offset >= 12,
        "sanity: mutation offset must be outside the adaptation field"
    );
    let mut mutated = duplicate;
    mutated[mutate_offset] ^= 0xFF;
    let cc_errors_after_mutation = cc_error_count(&[original, mutated]);
    assert_eq!(
        cc_errors_after_mutation, 1,
        "a same-CC repeat with a genuinely different payload byte must raise \
         exactly one Continuity_count_error — this is the #956 regression check"
    );

    // Restore: undo the mutation and confirm the pass state returns.
    let mut restored = mutated;
    restored[mutate_offset] ^= 0xFF;
    assert_eq!(
        restored, duplicate,
        "sanity: restore must reproduce the PCR-only duplicate"
    );
    let cc_errors_after_restore = cc_error_count(&[original, restored]);
    assert_eq!(
        cc_errors_after_restore, 0,
        "restoring the mutated byte must return to the legal-duplicate (no error) state"
    );
}
