//! Repair continuity counters in a TS file.
//!
//! Reads `m6-single.ts` (the test fixture) in bounded chunks, applies
//! `repair_continuity()`, and prints the number of continuity-counter
//! anomalies before and after repair. The capture is never held whole.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example repair_continuity
//! ```

use mpeg_ts::pid::Pid;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};

/// Path to the test fixture, resolved at compile time.
const FIXTURE_PATH: &str = "ts/m6-single.ts";

/// Length of one MPEG-2 TS packet (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;

/// Read granularity for streaming the fixture.
const READ_CHUNK_LEN: usize = 1 << 20;

/// Fill `packet` from `reader`; `false` at a clean end of stream.
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
    let path = test_fixtures::path(FIXTURE_PATH);
    let file = File::open(&path).expect("fixture m6-single.ts not found");
    let mut reader = io::BufReader::with_capacity(READ_CHUNK_LEN, file);

    // --- Before: count anomalies in the original ---
    let mut before = CcCounter::default();
    let mut input_packets = 0usize;

    // --- Apply repair ---
    let mut engine = ts_fix::TsFix::builder()
        .repair_continuity()
        .build()
        .expect("build should not fail");
    let mut after = CcCounter::default();
    let mut output_packets = 0usize;

    let mut packet = [0u8; TS_PACKET_LEN];
    while read_packet(&mut reader, &mut packet).expect("fixture is readable") {
        input_packets += 1;
        before.feed(&packet);
        engine
            .push(&packet, |emitted| {
                output_packets += 1;
                after.feed(emitted);
            })
            .expect("valid 188-byte packet");
    }
    engine.finish(|emitted| {
        output_packets += 1;
        after.feed(emitted);
    });

    println!(
        "input:      {} packets from {}",
        input_packets,
        path.display()
    );
    println!("output:     {output_packets} packets");
    println!(
        "CC anomalies before: {}, after: {}",
        before.anomalies, after.anomalies
    );
    println!(
        "result: {}",
        if after.anomalies == 0 { "PASS" } else { "FAIL" }
    );
}

/// Counts continuity-counter anomalies in a TS packet stream.
///
/// An anomaly is any payload-bearing packet whose CC does not match
/// `(last_cc + 1) & 0x0F` for that PID.
#[derive(Default)]
struct CcCounter {
    /// Last CC seen per PID.
    per_pid_cc: BTreeMap<Pid, u8>,
    /// Anomalies counted so far.
    anomalies: usize,
}

impl CcCounter {
    /// Examine one 188-byte packet.
    fn feed(&mut self, packet: &[u8]) {
        if packet.len() < 4 {
            return;
        }
        let pid = (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16;
        let cc = packet[3] & 0x0F;
        let has_payload = (packet[3] & 0x10) != 0;

        if has_payload {
            if let Some(&last_cc) = self.per_pid_cc.get(&Pid::from_wire_masked(pid)) {
                let expected = (last_cc + 1) & 0x0F;
                if cc != expected {
                    self.anomalies += 1;
                }
            }
            self.per_pid_cc.insert(Pid::from_wire_masked(pid), cc);
        }
    }
}
