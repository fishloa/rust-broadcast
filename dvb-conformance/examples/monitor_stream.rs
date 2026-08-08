//! Basic: run the TR 101 290 conformance monitor over a capture and print the
//! headline stats.
//!
//! Run with: `cargo run -p dvb-conformance --example monitor_stream`
//!
//! Reads the committed `m6-single.ts` fixture from the workspace-shared
//! `fixtures/` tree at runtime.

use core::time::Duration;
use dvb_conformance::ConformanceMonitor;

const PKT: usize = 188;
const INTER_PACKET_US: u64 = 40; // ~nominal spacing for the timing checks

fn main() {
    // Fixtures live in the workspace-shared `fixtures/` tree, not under a
    // sibling crate. A committed fixture that cannot be read is a bug, not a
    // reason to skip — so a missing/unreadable fixture is a hard failure.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts");
    let data = std::fs::read(path)
        .unwrap_or_else(|e| panic!("committed fixture {path} could not be read: {e}"));

    let mut monitor = ConformanceMonitor::new();
    let mut total_events = 0usize;
    for (i, pkt) in data.chunks(PKT).enumerate() {
        if pkt.len() < PKT {
            break;
        }
        let t = Duration::from_micros(i as u64 * INTER_PACKET_US);
        total_events += monitor.feed(pkt, t).len();
    }

    let stats = monitor.stats();
    println!("packets analysed : {}", stats.packets);
    println!("in sync          : {}", stats.in_sync);
    println!("events raised    : {total_events}");
}
