//! `check_ts` — run the Transport Stream diagnostics over a capture.
//!
//! Feeds a TS file through the pluggable [`Diagnostic`] harness one packet at
//! a time ([`DiagnosticRunner`]): sync byte, PAT/PMT versioning,
//! continuity-counter anomalies, PCR and PTS/DTS checks. The capture is read
//! in bounded chunks, never whole. Defaults to the committed
//! `fixtures/ts/h264-aac.ts`; pass a path to check your own capture:
//!
//! ```text
//! cargo run -p media-doctor --example check_ts [capture.ts]
//! ```
//!
//! Prints a per-rule tally and the first few findings. Findings on a real
//! capture are information, not failure, so the example exits 0 unless the
//! file cannot be read or holds no whole 188-byte packet.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};

use media_doctor::{
    CcAnomalyCheck, Diagnostic, DiagnosticRunner, PatPmtVersionCheck, PcrCheck, PmtStreamIndex,
    PtsCheck, Report, SyncByteCheck,
};

const TS_PACKET_LEN: usize = 188;
const SHOWN: usize = 5;

/// Read granularity for streaming the capture.
const READ_CHUNK_LEN: usize = 1 << 20;

/// Fill `packet` from `reader`; `false` at a clean end of stream (a trailing
/// partial packet is discarded, as the whole-buffer harness did).
fn read_packet(reader: &mut impl Read, packet: &mut [u8; TS_PACKET_LEN]) -> io::Result<bool> {
    let mut filled = 0usize;
    while filled < packet.len() {
        let read = reader.read(&mut packet[filled..])?;
        if read == 0 {
            return Ok(false);
        }
        filled += read;
    }
    Ok(true)
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        test_fixtures::path("ts/h264-aac.ts")
            .to_string_lossy()
            .into_owned()
    });
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            std::process::exit(2);
        }
    };
    let mut reader = io::BufReader::with_capacity(READ_CHUNK_LEN, file);

    let checks: [&dyn Diagnostic; 5] = [
        &SyncByteCheck,
        &PatPmtVersionCheck,
        &CcAnomalyCheck,
        &PcrCheck,
        &PtsCheck,
    ];
    let mut runner = DiagnosticRunner::new(&checks, &PmtStreamIndex::empty());
    let mut report = Report::new();
    let mut packets = 0usize;
    let mut packet = [0u8; TS_PACKET_LEN];
    loop {
        match read_packet(&mut reader, &mut packet) {
            Ok(true) => {
                packets += 1;
                runner.feed(&packet);
            }
            Ok(false) => break,
            Err(e) => {
                eprintln!("failed to read {path}: {e}");
                std::process::exit(2);
            }
        }
    }
    runner.finish(&mut report);
    if packets == 0 {
        eprintln!("{path}: shorter than one {TS_PACKET_LEN}-byte TS packet");
        std::process::exit(2);
    }

    println!("{path}: {} packets, {} finding(s)", packets, report.len());
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
