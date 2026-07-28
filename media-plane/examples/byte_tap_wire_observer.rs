//! [`ByteTap`](media_plane::ByteTap) observing every 188-byte packet of a
//! real broadcast capture at [`TapPoint::Wire`](media_plane::TapPoint::Wire)
//! -- the module's stated primary use case: letting analysis
//! (`dvb-conformance`'s `ConformanceMonitor`, `media-doctor watch`) see
//! bytes exactly as they arrived, before any demuxer gets a chance to reject
//! or discard them.
//!
//! Reads `fixtures/ts/h264_aac.ts` (a real broadcast capture shared across
//! this workspace's crates) via [`std::fs::read`], records every packet with
//! an arrival timestamp derived from its position in the file, then drains
//! the tap through a deliberately small ring so the run demonstrates both
//! halves of the tap contract on real data: ordinary delivery, and a
//! [`TapItem::Lagged`](media_plane::TapItem::Lagged) report when the
//! (simulated) consumer falls behind a real packet stream.
//!
//! ```text
//! cargo run -p media-plane --example byte_tap_wire_observer
//! ```

use bytes::Bytes;
use media_plane::{ByteTap, TapItem, TapPoint};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
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
    let bytes = std::fs::read(FIXTURE).expect("read the committed real-capture TS fixture");
    assert_eq!(
        bytes.len() % TS_PACKET_SIZE,
        0,
        "fixture must be a whole number of 188-byte TS packets"
    );
    let packet_count = bytes.len() / TS_PACKET_SIZE;
    println!(
        "byte_tap_wire_observer: read {} bytes ({packet_count} TS packets) from {FIXTURE}",
        bytes.len()
    );

    let mut tap = ByteTap::new(TapPoint::Wire, TAP_CAPACITY);

    let mut delivered = 0u64;
    let mut lagged_total = 0u64;
    let mut lagged_reports = 0u64;

    for (i, packet) in bytes.chunks(TS_PACKET_SIZE).enumerate() {
        tap.record(
            Bytes::copy_from_slice(packet),
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
    }
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
