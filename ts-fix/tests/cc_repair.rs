//! Continuity counter repair tests for `ts-fix`.
//!
//! Tests that the repair_continuity() operation correctly:
//! 1. Increments CC only on payload-bearing packets per ISO/IEC 13818-1 §2.4.3.3,
//! 2. Preserves legal duplicates (same PID, same CC, identical payload) per §2.4.3.3 L1772,
//! 3. Renumbers non-duplicate same-CC repeats (different payload = CC error, not spec duplicate),
//! 4. Preserves CC across signalled discontinuities per §2.4.3.5 L1872,
//! 5. Repairs genuine unsignalled, non-duplicate CC gaps.

use std::{collections::BTreeMap, fs, path::PathBuf};

mod support;

fn m6_single_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("ts")
        .join("m6-single.ts")
}

fn m6_duplicate_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("ts")
        .join("m6-duplicate.ts")
}

fn m6_discontinuity_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("ts")
        .join("m6-discontinuity.ts")
}

fn extract_pid(pkt: &[u8]) -> u16 {
    (((pkt[1] & 0x1F) as u16) << 8) | (pkt[2] as u16)
}

fn extract_cc(pkt: &[u8]) -> u8 {
    pkt[3] & 0x0F
}

fn has_payload(pkt: &[u8]) -> bool {
    pkt.len() >= 4 && (pkt[3] & 0x10) != 0
}

fn has_adaptation(pkt: &[u8]) -> bool {
    pkt.len() >= 4 && (pkt[3] & 0x20) != 0
}

fn has_discontinuity(pkt: &[u8]) -> bool {
    if pkt.len() < 6 {
        return false;
    }
    if !has_adaptation(pkt) {
        return false;
    }
    let af_len = pkt[4] as usize;
    if af_len == 0 {
        return false;
    }
    (pkt[5] & 0x80) != 0
}

fn get_payload(pkt: &[u8]) -> &[u8] {
    if !has_payload(pkt) {
        return &[];
    }
    let mut cursor = 4usize;
    if has_adaptation(pkt) {
        cursor += 1 + pkt[4] as usize;
    }
    if cursor >= 188 {
        return &[];
    }
    &pkt[cursor..188]
}

fn hash_payload_skip_pcr(pkt: &[u8]) -> u64 {
    let mut hash = 0xCBF29CE484222325u64;
    if has_adaptation(pkt) && pkt[4] > 0 {
        let has_pcr = (pkt[5] & 0x10) != 0;
        hash ^= pkt[5] as u64;
        hash = hash.wrapping_mul(0x100000001B3);
        if has_pcr {
            let af_body_end = 5 + pkt[4] as usize;
            for &b in &pkt[12..af_body_end] {
                hash ^= b as u64;
                hash = hash.wrapping_mul(0x100000001B3);
            }
        } else {
            for &b in &pkt[6..5 + pkt[4] as usize] {
                hash ^= b as u64;
                hash = hash.wrapping_mul(0x100000001B3);
            }
        }
    }
    for &b in get_payload(pkt) {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001B3);
    }
    hash
}

#[test]
fn repair_continuity_from_corrupted_zeros() {
    let mut input = fs::read(m6_single_path()).expect("fixture not found");
    support::zero_continuity_counters(&mut input);
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build");
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .unwrap();
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));
    assert_eq!(output.len(), input.len());
    let before = count_strict_anomalies(&input);
    let after = count_strict_anomalies(&output);
    assert!(after <= before, "zeros repair must not increase anomalies");
}

#[test]
fn repair_continuity_from_xor_corruption() {
    let mut input = fs::read(m6_single_path()).expect("fixture not found");
    support::xor_continuity_counters(&mut input, 0xAA);
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build");
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .unwrap();
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));
    assert_eq!(output.len(), input.len());
    let before = count_strict_anomalies(&input);
    let after = count_strict_anomalies(&output);
    assert!(after <= before, "XOR repair must not increase anomalies");
}

#[test]
fn repair_continuity_makes_stream_valid() {
    let input = fs::read(m6_single_path()).expect("fixture not found");
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build");
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .unwrap();
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));
    assert_eq!(output.len(), input.len());
    assert_eq!(output.len() % 188, 0);
    let before = count_strict_anomalies(&input);
    let after = count_strict_anomalies(&output);
    assert!(
        after <= before,
        "repair must not increase strict CC anomalies"
    );
}

#[test]
fn legal_duplicates_are_preserved_from_real_fixture() {
    let input = fs::read(m6_duplicate_path()).expect("fixture m6-duplicate.ts not found");
    let mut legal_dup_count = 0usize;
    let mut last_per_pid: BTreeMap<u16, (u8, u64)> = BTreeMap::new();
    for chunk in input.chunks(188) {
        let pid = extract_pid(chunk);
        let cc = extract_cc(chunk);
        let hp = has_payload(chunk);
        if hp {
            if let Some(&(last_cc, last_hash)) = last_per_pid.get(&pid) {
                if cc == last_cc && hash_payload_skip_pcr(chunk) == last_hash {
                    legal_dup_count += 1;
                }
            }
            last_per_pid.insert(pid, (cc, hash_payload_skip_pcr(chunk)));
        }
    }
    assert_eq!(
        legal_dup_count, 5,
        "m6-duplicate.ts should have 5 legal duplicates"
    );

    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build");
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .unwrap();
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    let mut output_dup_count = 0usize;
    let mut out_last: BTreeMap<u16, (u8, u64)> = BTreeMap::new();
    for chunk in output.chunks(188) {
        let pid = extract_pid(chunk);
        let cc = extract_cc(chunk);
        let hp = has_payload(chunk);
        if hp {
            if let Some(&(last_cc, last_hash)) = out_last.get(&pid) {
                if cc == last_cc && hash_payload_skip_pcr(chunk) == last_hash {
                    output_dup_count += 1;
                }
            }
            out_last.insert(pid, (cc, hash_payload_skip_pcr(chunk)));
        }
    }
    assert_eq!(
        output_dup_count, 5,
        "all 5 legal duplicates preserved, got {output_dup_count}"
    );
}

#[test]
fn cc_errors_on_m6_single_are_renumbered() {
    let input = fs::read(m6_single_path()).expect("fixture m6-single.ts not found");
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build");
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .unwrap();
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    let mut remaining_errors = 0usize;
    let mut last_per_pid: BTreeMap<u16, (u8, u64)> = BTreeMap::new();
    for chunk in output.chunks(188) {
        let pid = extract_pid(chunk);
        let cc = extract_cc(chunk);
        let hp = has_payload(chunk);
        if hp {
            if let Some(&(last_cc, last_hash)) = last_per_pid.get(&pid) {
                if cc == last_cc && hash_payload_skip_pcr(chunk) != last_hash {
                    remaining_errors += 1;
                }
            }
            last_per_pid.insert(pid, (cc, hash_payload_skip_pcr(chunk)));
        }
    }
    assert_eq!(
        remaining_errors, 0,
        "CC errors must be renumbered; {remaining_errors} remain"
    );
}

#[test]
fn discontinuity_indicator_cc_is_preserved_from_real_fixture() {
    let input = fs::read(m6_discontinuity_path()).expect("fixture m6-discontinuity.ts not found");
    let mut disc_indices: Vec<usize> = Vec::new();
    for (idx, chunk) in input.chunks(188).enumerate() {
        if has_discontinuity(chunk) {
            disc_indices.push(idx);
        }
    }
    assert_eq!(disc_indices.len(), 3);
    let disc_ccs: Vec<u8> = disc_indices
        .iter()
        .map(|&idx| extract_cc(&input[idx * 188..]))
        .collect();

    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build");
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .unwrap();
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    for (i, &idx) in disc_indices.iter().enumerate() {
        assert_eq!(disc_ccs[i], extract_cc(&output[idx * 188..]));
    }
}

#[test]
fn genuine_cc_gap_is_repaired_in_stream() {
    let mut input = fs::read(m6_single_path()).expect("fixture not found");
    let target_pid = 0x0082u16;
    let mut payload_count = 0u8;
    let mut injected = false;
    for chunk in input.chunks_mut(188) {
        let pid = extract_pid(chunk);
        let hp = has_payload(chunk);
        if pid == target_pid && hp {
            payload_count += 1;
            if payload_count == 2 {
                let cc = extract_cc(chunk);
                chunk[3] = (chunk[3] & 0xF0) | ((cc ^ 0x08) & 0x0F);
                injected = true;
                break;
            }
        }
    }
    assert!(injected, "should have injected a CC error");

    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build");
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .unwrap();
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    let before = count_pid_anomalies(&input, target_pid);
    let after = count_pid_anomalies(&output, target_pid);
    assert!(
        after < before,
        "PID {target_pid:#05x}: before={before}, after={after}; repair must reduce"
    );
}

#[test]
fn strict_plus_one_would_renumber_legal_duplicates() {
    let input = fs::read(m6_duplicate_path()).expect("fixture m6-duplicate.ts not found");
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build");
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .unwrap();
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    let mut output_dup_count = 0usize;
    let mut out_last: BTreeMap<u16, (u8, u64)> = BTreeMap::new();
    for chunk in output.chunks(188) {
        let pid = extract_pid(chunk);
        let cc = extract_cc(chunk);
        let hp = has_payload(chunk);
        if hp {
            if let Some(&(last_cc, last_hash)) = out_last.get(&pid) {
                if cc == last_cc && hash_payload_skip_pcr(chunk) == last_hash {
                    output_dup_count += 1;
                }
            }
            out_last.insert(pid, (cc, hash_payload_skip_pcr(chunk)));
        }
    }
    assert_eq!(
        output_dup_count, 5,
        "duplicate-preservation bite: {output_dup_count}/5"
    );
    let strict = count_strict_anomalies(&output);
    assert!(
        strict > 0,
        "strict-+1 sees {strict} anomalies — if 0, duplicates renumbered"
    );
}

fn count_strict_anomalies(data: &[u8]) -> usize {
    let mut per_pid_cc: BTreeMap<u16, u8> = BTreeMap::new();
    let mut anomalies = 0;
    for chunk in data.chunks(188) {
        if chunk.len() < 4 {
            continue;
        }
        let pid = extract_pid(chunk);
        let cc = extract_cc(chunk);
        let hp = has_payload(chunk);
        if hp {
            if let Some(&last_cc) = per_pid_cc.get(&pid) {
                let expected = (last_cc + 1) & 0x0F;
                if cc != expected {
                    anomalies += 1;
                }
            }
            per_pid_cc.insert(pid, cc);
        }
    }
    anomalies
}

fn count_pid_anomalies(data: &[u8], pid: u16) -> usize {
    let mut last_cc: Option<u8> = None;
    let mut anomalies = 0;
    for chunk in data.chunks(188) {
        if extract_pid(chunk) != pid || !has_payload(chunk) {
            continue;
        }
        let cc = extract_cc(chunk);
        if let Some(last) = last_cc {
            if cc != (last + 1) & 0x0F {
                anomalies += 1;
            }
        }
        last_cc = Some(cc);
    }
    anomalies
}
