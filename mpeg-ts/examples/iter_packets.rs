//! Walk a TS file using the `iter_packets` helper, printing typed field accessors.
//!
//! Demonstrates the [`ScramblingControl`] and [`AdaptationFieldControl`] typed enums
//! and the `iter_packets` bulk-walk helper introduced in `mpeg-ts` 0.2.0.
//!
//! Run with:
//!   cargo run -p mpeg-ts --example iter_packets
//!   cargo run -p mpeg-ts --example iter_packets -- path/to/stream.ts

use std::collections::HashMap;
use std::env;
use std::fs;

use mpeg_ts::ts::{ScramblingControl, iter_packets};

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "mpeg-ts/tests/fixtures/m6-single.ts".to_string());

    let buf = fs::read(&path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {path}: {e}");
        std::process::exit(1);
    });

    let mut total = 0u64;
    let mut scrambled = 0u64;
    let mut pid_counts: HashMap<u16, u64> = HashMap::new();

    for pkt in iter_packets(&buf) {
        total += 1;
        let hdr = &pkt.header;
        *pid_counts.entry(hdr.pid).or_insert(0) += 1;

        let sc = hdr.scrambling_control();
        let afc = hdr.adaptation_field_control();

        if sc != ScramblingControl::NotScrambled {
            scrambled += 1;
        }

        if total <= 5 || hdr.pusi {
            println!(
                "pkt {:>6}: pid=0x{:04X} cc={:2} scrambling={} afc={}",
                total, hdr.pid, hdr.continuity_counter, sc, afc,
            );
        }
    }

    let clear = total - scrambled;
    println!("\n--- summary ---");
    println!("total packets : {total}");
    println!("clear packets : {clear}");
    println!("scrambled pkts: {scrambled}");
    println!("distinct PIDs : {}", pid_counts.len());

    if total == 0 {
        eprintln!("warning: no packets found in {path}");
        std::process::exit(1);
    }
}
