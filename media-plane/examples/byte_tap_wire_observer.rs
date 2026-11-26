//! [`ByteTap`](media_plane::ByteTap) observing every 188-byte packet of a
//! real broadcast capture at [`TapPoint::Wire`](media_plane::TapPoint::Wire)
//! -- the module's stated primary use case: letting analysis
//! (`dvb-conformance`'s `ConformanceMonitor`, `media-doctor watch`) see
//! bytes exactly as they arrived, before any demuxer gets a chance to reject
//! or discard them.
//!
//! Streams `fixtures/ts/h264-aac.ts` (a real broadcast capture shared across
//! this workspace's crates) one 188-byte packet at a time, records every
//! packet with an arrival timestamp derived from its position in the file,
//! then drains the tap through a deliberately small ring so the run
//! demonstrates both halves of the tap contract on real data: ordinary
//! delivery, and a [`TapItem::Lagged`](media_plane::TapItem::Lagged) report
//! when the (simulated) consumer falls behind a real packet stream.
//!
//! ```text
//! cargo run -p media-plane --example byte_tap_wire_observer
//! ```

use std::fs::File;
use std::io::{self, BufReader, Read};

use bytes::Bytes;
use media_plane::{ByteTap, TapItem, TapPoint};

const FIXTURE: &str = "ts/h264-aac.ts";
const TS_PACKET_SIZE: usize = 188;
/// Deliberately small next to a real capture's packet count, so this run
/// exercises the tap's eviction/`Lagged` path on genuine data, not just the
/// plain-delivery path a larger ring would only ever show.
const TAP_CAPACITY: usize = 32;
/// How many packets the simulated consumer lets accumulate before it polls
/// again -- larger than `TAP_CAPACITY` so eviction is guaranteed to happen.
const CONSUMER_POLL_STRIDE: usize = 64;

use broadcast_common::stage::Timestamp;

fn main() {
    let path = test_fixtures::path(FIXTURE);
    let file = File::open(&path).unwrap_or_else(|e| {
        panic!(
            "committed fixture {} could not be read: {e}",
            path.display()
        )
    });
    let mut reader = BufReader::new(file);
    let mut packet = [0u8; TS_PACKET_SIZE];

    let mut tap = ByteTap::new(TapPoint::Wire, TAP_CAPACITY)
        .unwrap_or_else(|e| panic!("a non-zero tap capacity is built: {e}"));

    let mut delivered = 0u64;
    let mut lagged_total = 0u64;
    let mut lagged_reports = 0u64;
    let mut packet_count = 0usize;

    loop {
        let read = read_packet(&mut reader, &mut packet).unwrap_or_else(|e| {
            panic!(
                "committed fixture {} could not be read: {e}",
                path.display()
            )
        });
        if read == 0 {
            break;
        }
        assert_eq!(
            read, TS_PACKET_SIZE,
            "fixture must be a whole number of 188-byte TS packets"
        );
        let i = packet_count;
        tap.record(
            Bytes::copy_from_slice(&packet),
            Timestamp::from_nanos(i as u64),
        );

        // The simulated consumer only checks in every `CONSUMER_POLL_STRIDE`
        // packets -- a real conformance monitor that briefly falls behind
        // live ingest, not a producer that ever blocks (`record` never
        // does).
        if i % CONSUMER_POLL_STRIDE == 0 {
            while let Some(item) = tap.poll() {
                match item {
                    TapItem::Data(_, _) => delivered += 1,
                    TapItem::Lagged { skipped } => {
                        lagged_total += skipped;
                        lagged_reports += 1;
                    }
                    // `TapItem` is `#[non_exhaustive]`: a future loss class
                    // (the escalated `Degraded` the sample ring already
                    // distinguishes, say) must not silently count as
                    // delivered data.
                    other => panic!("unhandled TapItem variant: {other:?}"),
                }
            }
        }
        packet_count += 1;
    }

    println!(
        "byte_tap_wire_observer: read {} bytes ({packet_count} TS packets) from {FIXTURE}",
        packet_count * TS_PACKET_SIZE
    );
    // Final drain.
    while let Some(item) = tap.poll() {
        match item {
            TapItem::Data(_, _) => delivered += 1,
            TapItem::Lagged { skipped } => {
                lagged_total += skipped;
                lagged_reports += 1;
            }
            other => panic!("unhandled TapItem variant: {other:?}"),
        }
    }

    println!(
        "byte_tap_wire_observer: {delivered} packet(s) delivered, {lagged_reports} Lagged report(s) totalling {lagged_total} skipped packet(s)"
    );
    assert_eq!(
        delivered + lagged_total,
        packet_count as u64,
        "every real packet must be accounted for as delivered or skipped"
    );
    assert!(
        lagged_total > 0,
        "this run's ring/stride are chosen so a real capture this size must lag at least once"
    );
}

/// Fill `buf` from `reader`; short only at end of stream.
fn read_packet(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}
