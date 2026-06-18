//! Advanced: run the TR 101 290 monitor and break the findings down by
//! measurement priority and indicator — the shape of a real QoS report.
//!
//! Run with: `cargo run -p dvb-conformance --example priority_breakdown`

use core::time::Duration;
use dvb_conformance::{ConformanceEvent, ConformanceMonitor, Priority};
use std::collections::BTreeMap;

const PKT: usize = 188;
const INTER_PACKET_US: u64 = 40;

fn main() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../dvb-si/tests/fixtures/m6-single.ts"
    );
    let data = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    let mut monitor = ConformanceMonitor::new();
    let mut events: Vec<ConformanceEvent> = Vec::new();
    for (i, pkt) in data.chunks(PKT).enumerate() {
        if pkt.len() < PKT {
            break;
        }
        let t = Duration::from_micros(i as u64 * INTER_PACKET_US);
        events.extend_from_slice(monitor.feed(pkt, t));
    }

    let (mut p1, mut p2, mut p3) = (0u32, 0u32, 0u32);
    let mut by_indicator: BTreeMap<String, u32> = BTreeMap::new();
    for ev in &events {
        match ev.priority {
            Priority::First => p1 += 1,
            Priority::Second => p2 += 1,
            Priority::Third => p3 += 1,
            _ => {}
        }
        *by_indicator.entry(ev.indicator.to_string()).or_default() += 1;
    }

    let stats = monitor.stats();
    println!("== TR 101 290 report ({} packets) ==", stats.packets);
    println!("Priority 1 (must decode) : {p1}");
    println!("Priority 2 (recommended) : {p2}");
    println!("Priority 3 (application) : {p3}");
    if by_indicator.is_empty() {
        println!("\nno conformance events — stream is clean ✔");
    } else {
        println!("\nby indicator:");
        for (name, n) in &by_indicator {
            println!("  {name:<24} {n}");
        }
    }
}
