//! `check_ts` — run the Transport Stream diagnostics over a capture.
//!
//! Feeds a TS file through the pluggable [`Diagnostic`] harness
//! (`run_all`): sync byte, PAT/PMT versioning, continuity-counter anomalies,
//! PCR and PTS/DTS checks. Defaults to the committed
//! `fixtures/ts/h264_aac.ts`; pass a path to check your own capture:
//!
//! ```text
//! cargo run -p media-doctor --example check_ts [capture.ts]
//! ```
//!
//! Prints a per-rule tally and the first few findings. Findings on a real
//! capture are information, not failure, so the example exits 0 unless the
//! file cannot be read or holds no whole 188-byte packet.

use std::collections::BTreeMap;

use media_doctor::{
    CcAnomalyCheck, Diagnostic, PatPmtVersionCheck, PcrCheck, PtsCheck, Report, SyncByteCheck,
    run_all,
};

const TS_PACKET_LEN: usize = 188;
const SHOWN: usize = 5;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts").to_string()
    });
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            std::process::exit(2);
        }
    };
    // The harness takes whole packets; drop any trailing partial one.
    let whole = data.len() / TS_PACKET_LEN * TS_PACKET_LEN;
    if whole == 0 {
        eprintln!("{path}: shorter than one {TS_PACKET_LEN}-byte TS packet");
        std::process::exit(2);
    }

    let checks: [&dyn Diagnostic; 5] = [
        &SyncByteCheck,
        &PatPmtVersionCheck,
        &CcAnomalyCheck,
        &PcrCheck,
        &PtsCheck,
    ];
    let mut report = Report::new();
    run_all(&data[..whole], &checks, &mut report);

    println!(
        "{path}: {} packets, {} finding(s)",
        whole / TS_PACKET_LEN,
        report.len()
    );
    let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for f in report.findings() {
        *by_rule.entry(f.rule_id.as_str()).or_default() += 1;
    }
    for (rule, n) in &by_rule {
        println!("    {rule}: {n}");
    }
    for f in report.findings().iter().take(SHOWN) {
        println!("  {f}");
    }
}
