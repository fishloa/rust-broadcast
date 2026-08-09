//! Demonstrate the `media-plane` attachment point a live host actually
//! uses: a [`media_plane::ByteTap`] fed with raw TS bytes (standing in for
//! whatever a real `Dialer`/`Listener` records at the wire), drained through
//! [`compliance_probe::Probe::drain_byte_tap`]. No socket is opened —
//! reading a committed fixture and recording it into the tap is enough to
//! exercise the exact code path a live probe uses.
//!
//! Run with: `cargo run -p compliance-probe --example byte_tap_live`

use std::fs;

use broadcast_common::Timestamp;
use bytes::Bytes;
use compliance_probe::Probe;
use compliance_probe::trunk_bridge::TrunkBridge;
use media_plane::{ByteTap, TapPoint};
use mpeg_ts::ts::TS_PACKET_SIZE;

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts");
    let data = fs::read(path)
        .unwrap_or_else(|e| panic!("committed fixture {path} could not be read: {e}"));

    // A real ingest driver calls `ByteTap::record` once per arriving
    // datagram/read and a probe host calls `drain_byte_tap` periodically —
    // *not* "record everything, then drain once" (a real live probe never
    // reaches "everything"). This interleaves the two the same way, one
    // "datagram" (seven 188-byte TS packets, `media-doctor watch`'s own
    // convention) at a time, so the tap's bounded ring never needs to hold
    // more than a handful of datagrams at once.
    let mut tap = ByteTap::new(TapPoint::Wire, 16);
    let mut probe = Probe::new();
    let mut bridge = TrunkBridge::default();
    let chunk_size = 7 * TS_PACKET_SIZE;
    for (i, chunk) in data.chunks(chunk_size).enumerate() {
        tap.record(
            Bytes::copy_from_slice(chunk),
            Timestamp::from_nanos(i as u64 * 1_000_000),
        );
        probe.drain_byte_tap(&mut bridge, &mut tap, Timestamp::ZERO);
    }

    let stats = probe.conformance_stats();
    println!("packets analysed via ByteTap: {}", stats.packets);
    println!("in sync                     : {}", stats.in_sync);
}
