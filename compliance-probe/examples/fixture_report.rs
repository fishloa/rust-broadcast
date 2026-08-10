//! Run the probe over a committed real-capture fixture and print a headline
//! report, cross-checked against the same fixture's already-known-good
//! `dvb-conformance` numbers.
//!
//! Run with: `cargo run -p compliance-probe --example fixture_report`
//!
//! # Why 40 µs per packet, and why that choice is not arbitrary
//!
//! A file has no arrival timing of its own, so replaying one requires
//! *assuming* a bitrate — and per the crate docs' "The clock you feed is part
//! of the measurement", that assumption changes the answer for every
//! timeout-based indicator. 40 µs per 188-byte packet is ~37.6 Mbit/s, a full
//! DVB multiplex rate. `tests/wasm_analyzer_equivalence.rs` measures the
//! sensitivity directly: this fixture reports a stable 876 events anywhere
//! from ~100 µs/packet up to ~1.5 ms/packet, so 40 µs sits on that plateau
//! rather than near a boundary where the number would move.

use std::time::Duration;

use compliance_probe::Probe;
use mpeg_ts::ts::TS_PACKET_SIZE;

/// Assumed inter-packet arrival interval. See this example's module docs for
/// why this specific value, and why the choice matters at all.
const INTER_PACKET_MICROS: u64 = 40;

fn main() {
    // Fixtures live in the workspace-shared `fixtures/` tree, not under a
    // sibling crate — see `dvb-conformance/examples/monitor_stream.rs` for
    // the same pattern this example mirrors.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts");
    let data = std::fs::read(path)
        .unwrap_or_else(|e| panic!("committed fixture {path} could not be read: {e}"));

    let mut probe = Probe::new();
    for (i, chunk) in data.chunks(TS_PACKET_SIZE).enumerate() {
        if chunk.len() < TS_PACKET_SIZE {
            break;
        }
        let mut packet = [0u8; TS_PACKET_SIZE];
        packet.copy_from_slice(chunk);
        let t = Duration::from_micros(i as u64 * INTER_PACKET_MICROS);
        probe.feed_ts_packet(&packet, t);
    }

    let stats = probe.conformance_stats();
    println!("packets analysed : {}", stats.packets);
    println!("in sync          : {}", stats.in_sync);
    println!("TR 101 290 events: {}", stats.events);
    println!(
        "(assumed arrival : {INTER_PACKET_MICROS} us/packet -- the clock is an \
         input to this result, not bookkeeping; see the module docs)"
    );
}
