//! Demonstrate the `media-plane` attachment point a live host actually
//! uses: a [`media_plane::ByteTap`] fed with raw TS bytes (standing in for
//! whatever a real `Dialer`/`Listener` records at the wire), drained through
//! [`compliance_probe::Probe::drain_byte_tap`]. No socket is opened —
//! reading a committed fixture and recording it into the tap is enough to
//! exercise the exact code path a live probe uses.
//!
//! Run with: `cargo run -p compliance-probe --example byte_tap_live`

use std::fs::File;
use std::io::{self, BufReader, Read};

use broadcast_common::Timestamp;
use bytes::Bytes;
use compliance_probe::Probe;
use compliance_probe::trunk_bridge::TrunkBridge;
use media_plane::{ByteTap, TapPoint};
use mpeg_ts::ts::TS_PACKET_SIZE;

fn main() {
    let path = test_fixtures::path("ts/m6-single.ts");
    let file = File::open(&path).unwrap_or_else(|e| {
        panic!(
            "committed fixture {} could not be read: {e}",
            path.display()
        )
    });

    // A real ingest driver calls `ByteTap::record` once per arriving
    // datagram/read and a probe host calls `drain_byte_tap` periodically —
    // *not* "record everything, then drain once" (a real live probe never
    // reaches "everything"). This interleaves the two the same way, one
    // "datagram" (seven 188-byte TS packets, `media-doctor watch`'s own
    // convention) at a time, so the tap's bounded ring never needs to hold
    // more than a handful of datagrams at once.
    let mut tap = ByteTap::new(TapPoint::Wire, 16)
        .unwrap_or_else(|e| panic!("a non-zero tap capacity is built: {e}"));
    let mut probe = Probe::new();
    let mut bridge = TrunkBridge::default();
    let chunk_size = 7 * TS_PACKET_SIZE;
    let mut reader = BufReader::with_capacity(chunk_size * 2, file);
    let mut chunk = vec![0u8; chunk_size];
    let mut index = 0u64;
    while let Some(bytes) = read_chunk(&mut reader, &mut chunk).unwrap_or_else(|e| {
        panic!(
            "committed fixture {} could not be read: {e}",
            path.display()
        )
    }) {
        tap.record(
            Bytes::copy_from_slice(bytes),
            Timestamp::from_nanos(index * 1_000_000),
        );
        probe.drain_byte_tap(&mut bridge, &mut tap, Timestamp::ZERO);
        index += 1;
    }

    let stats = probe.conformance_stats();
    println!("packets analysed via ByteTap: {}", stats.packets);
    println!("in sync                     : {}", stats.in_sync);
}

/// Read the next chunk; `None` at a clean end of stream.
fn read_chunk<'a>(reader: &mut impl Read, buf: &'a mut [u8]) -> io::Result<Option<&'a [u8]>> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok((filled > 0).then_some(&buf[..filled]))
}
