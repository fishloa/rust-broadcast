//! Repair continuity counters in a TS file.
//!
//! Reads `m6-single.ts` (the test fixture), applies `repair_continuity()`, and
//! prints the number of continuity-counter anomalies before and after repair.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example repair_continuity
//! ```

use std::collections::BTreeMap;
use std::fs;

/// Path to the test fixture, resolved at compile time.
const FIXTURE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m6-single.ts");

fn main() {
    let input = fs::read(FIXTURE_PATH).expect("fixture m6-single.ts not found");

    // --- Before: count anomalies in the original ---
    let before_anomalies = count_cc_anomalies(&input);
    println!(
        "input:      {} packets from {FIXTURE_PATH}",
        input.len() / 188
    );

    // --- Apply repair ---
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build should not fail");

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    // --- After: count anomalies in the repaired ---
    let after_anomalies = count_cc_anomalies(&output);

    println!("output:     {} packets", output.len() / 188);
    println!("CC anomalies before: {before_anomalies}, after: {after_anomalies}");
    println!(
        "result: {}",
        if after_anomalies == 0 { "PASS" } else { "FAIL" }
    );
}

/// Count continuity-counter anomalies in a TS byte stream.
///
/// An anomaly is any payload-bearing packet whose CC does not match
/// `(last_cc + 1) & 0x0F` for that PID.
fn count_cc_anomalies(data: &[u8]) -> usize {
    let mut per_pid_cc: BTreeMap<u16, u8> = BTreeMap::new();
    let mut anomalies = 0;

    for chunk in data.chunks(188) {
        if chunk.len() < 4 {
            continue;
        }
        let pid = (((chunk[1] & 0x1F) as u16) << 8) | chunk[2] as u16;
        let cc = chunk[3] & 0x0F;
        let has_payload = (chunk[3] & 0x10) != 0;

        if has_payload {
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
