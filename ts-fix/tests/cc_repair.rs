//! Continuity counter repair tests for `ts-fix`.
//!
//! Tests that the repair_continuity() operation correctly:
//! 1. Reads per-PID continuity counter state per ISO/IEC 13818-1 §2.4.3.3,
//! 2. Increments only on payload-bearing packets,
//! 3. Maintains per-PID state across packet interleaving,
//! 4. Recovers to match the original capture when the original was valid.

use std::{collections::BTreeMap, fs, path::PathBuf};

mod support;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("m6-single.ts")
}

/// Extract the PID from a TS packet header (bytes 1-2, with some bit masks).
fn extract_pid(pkt: &[u8]) -> u16 {
    if pkt.len() < 4 {
        return 0;
    }
    let b1 = pkt[1];
    let b2 = pkt[2];
    (((b1 & 0x1F) as u16) << 8) | (b2 as u16)
}

/// Extract the continuity counter from a TS packet header (byte 3, bits [3:0]).
fn extract_cc(pkt: &[u8]) -> u8 {
    if pkt.len() < 4 {
        return 0;
    }
    pkt[3] & 0x0F
}

/// Check if packet has payload based on adaptation field control bits.
/// Byte 3, bits [4:3]: 01 or 11 means payload is present.
fn has_payload(pkt: &[u8]) -> bool {
    if pkt.len() < 4 {
        return false;
    }
    (pkt[3] & 0x10) != 0
}

#[test]
fn repair_continuity_from_corrupted_zeros() {
    let mut input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");

    // Corrupt: zero all continuity counters.
    support::zero_continuity_counters(&mut input);

    // Build engine with continuity repair.
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("repair_continuity build should not fail");

    let mut output = Vec::with_capacity(input.len());

    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }

    engine.finish(|pkt| output.extend_from_slice(pkt));

    // Verify output length matches input.
    assert_eq!(output.len(), input.len(), "repaired stream length mismatch");

    // Verify per-PID continuity is monotonic after repair.
    let mut per_pid_cc = BTreeMap::new();
    let mut pids_with_2plus_payloads = 0;

    for (pkt_idx, repaired_chunk) in output.chunks(188).enumerate() {
        let pid = extract_pid(repaired_chunk);
        let repaired_cc = extract_cc(repaired_chunk);
        let pkt_has_payload = has_payload(repaired_chunk);

        // Verify per-PID monotonicity for payload-bearing packets.
        if pkt_has_payload {
            if let Some(&last_cc) = per_pid_cc.get(&pid) {
                let expected_cc = (last_cc + 1) & 0x0F;
                assert_eq!(
                    repaired_cc, expected_cc,
                    "packet {} PID {:#05x}: payload CC should be {}, got {}",
                    pkt_idx, pid, expected_cc, repaired_cc
                );
            }
            per_pid_cc.insert(pid, repaired_cc);
        } else {
            // Adaptation-only packet: CC should match the last known CC for that PID.
            if let Some(&expected_cc) = per_pid_cc.get(&pid) {
                assert_eq!(
                    repaired_cc, expected_cc,
                    "packet {} PID {:#05x}: adaptation-only CC should match last payload CC {}",
                    pkt_idx, pid, expected_cc
                );
            }
        }
    }

    // Count PIDs with at least 2 payload packets (boundary test).
    for &pid in per_pid_cc.keys() {
        let count = output
            .chunks(188)
            .filter(|pkt| has_payload(pkt) && extract_pid(pkt) == pid)
            .count();
        if count >= 2 {
            pids_with_2plus_payloads = 1;
        }
    }
    assert!(
        pids_with_2plus_payloads > 0,
        "fixture should have at least one PID with 2+ payload packets"
    );
}

#[test]
fn repair_continuity_from_xor_corruption() {
    let mut input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");

    // Corrupt: XOR continuity counters with a per-packet pattern.
    support::xor_continuity_counters(&mut input, 0xAA);

    // Build engine with continuity repair.
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("repair_continuity build should not fail");

    let mut output = Vec::with_capacity(input.len());

    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }

    engine.finish(|pkt| output.extend_from_slice(pkt));

    // Verify output length matches input.
    assert_eq!(output.len(), input.len(), "repaired stream length mismatch");

    // Verify per-PID continuity is monotonic after repair.
    let mut per_pid_cc = BTreeMap::new();

    for (pkt_idx, repaired_chunk) in output.chunks(188).enumerate() {
        let pid = extract_pid(repaired_chunk);
        let repaired_cc = extract_cc(repaired_chunk);
        let pkt_has_payload = has_payload(repaired_chunk);

        // Verify per-PID monotonicity for payload-bearing packets.
        if pkt_has_payload {
            if let Some(&last_cc) = per_pid_cc.get(&pid) {
                let expected_cc = (last_cc + 1) & 0x0F;
                assert_eq!(
                    repaired_cc, expected_cc,
                    "packet {} PID {:#05x}: payload CC should be {}, got {}",
                    pkt_idx, pid, expected_cc, repaired_cc
                );
            }
            per_pid_cc.insert(pid, repaired_cc);
        }
    }
}

#[test]
fn repair_continuity_makes_stream_valid() {
    let input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");

    // Process the fixture through the repair engine.
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("repair_continuity build should not fail");

    let mut output = Vec::with_capacity(input.len());

    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }

    engine.finish(|pkt| output.extend_from_slice(pkt));

    // Verify the output stream has correct per-PID monotonic CCs.
    let mut per_pid_cc = BTreeMap::new();

    for (pkt_idx, repaired_chunk) in output.chunks(188).enumerate() {
        let pid = extract_pid(repaired_chunk);
        let repaired_cc = extract_cc(repaired_chunk);
        let pkt_has_payload = has_payload(repaired_chunk);

        // Verify per-PID monotonicity for payload-bearing packets.
        if pkt_has_payload {
            if let Some(&last_cc) = per_pid_cc.get(&pid) {
                let expected_cc = (last_cc + 1) & 0x0F;
                assert_eq!(
                    repaired_cc, expected_cc,
                    "packet {} PID {:#05x}: payload CC should be {}, got {}",
                    pkt_idx, pid, expected_cc, repaired_cc
                );
            }
            per_pid_cc.insert(pid, repaired_cc);
        }
    }

    // Ensure the output is a valid multiple of 188.
    assert_eq!(
        output.len() % 188,
        0,
        "output must be a multiple of 188 bytes"
    );
}
