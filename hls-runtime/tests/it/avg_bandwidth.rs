#![cfg(feature = "std")]
//! `AVERAGE-BANDWIDTH` on the master playlist the sans-IO [`HlsOrigin`]
//! renders — Apple, *HLS Authoring Specification for Apple Devices*, item
//! 9.14 (MUST include it) and RFC 8216bis §4.4.6.2 (`decimal-integer`, the
//! average segment bit rate of the Variant Stream).
//!
//! The value is the mean bitrate over **every** segment the origin has
//! drained (`round(sum(bytes)*8 / sum(seconds))`), not just the ones still in
//! the playlist window, and it is clamped to `BANDWIDTH` (the average can
//! never exceed the peak). The byte sizes and durations below are deliberately
//! unequal, so a bytes/duration swap, a window-only average, or a
//! peak-substituted-as-average each produce a different number and fail.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_hls::MasterPlaylist;
use bytes::Bytes;
use hls_runtime::server::HlsOrigin;
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::SegmentMeta;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn trunk_with(segment_cap: usize) -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(TrunkConfig::new(
        nz(64),
        nz(8),
        nz(segment_cap),
        nz(8),
        nz(64),
    ));
    let writer = trunk.segment_writer().expect("first segment writer");
    (trunk, writer)
}

fn origin(trunk: &Arc<Trunk>, window: usize) -> HlsOrigin {
    HlsOrigin::builder(Arc::clone(trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(window))
        .instance(7)
        .build()
        .expect("origin builds")
}

/// Publish one closed segment of `bytes` length over `millis` milliseconds
/// carrying a repeating byte pattern (a real payload, not a constant filler,
/// so an off-by-a-run length cannot hide).
fn publish(w: &SegmentWriter, seq: u32, millis: u64, bytes: usize) {
    let payload: Vec<u8> = (0..bytes).map(|i| u8::try_from(i % 251).unwrap()).collect();
    w.publish_segment(SegmentEntry::new(
        Bytes::from(payload),
        SegmentSeq::new(seq),
        Duration::from_millis(millis),
        broadcast_common::Timestamp::from_nanos(u64::from(seq) * 1_000_000_000),
        SegmentMeta {
            discontinuous: false,
        },
    ))
    .expect("monotonic sequence number");
}

fn master(origin: &HlsOrigin) -> MasterPlaylist {
    let rendered = origin
        .master_playlist("media.m3u8")
        .expect("master renders");
    MasterPlaylist::parse(&rendered).expect("rendered master must re-parse")
}

fn assert_average(variant_avg: Option<u32>, bandwidth: u32, expected: u32, context: &str) {
    assert_eq!(
        variant_avg,
        Some(expected),
        "{context}: AVERAGE-BANDWIDTH must be {expected} (BANDWIDTH {bandwidth})"
    );
    assert!(
        expected < bandwidth,
        "{context}: the fixture must make average < BANDWIDTH, got {expected} vs {bandwidth}"
    );
}

#[test]
fn average_bandwidth_is_the_mean_of_two_unequal_segments() {
    let (trunk, w) = trunk_with(8);
    let o = origin(&trunk, 4);
    // Segment 1: 900_000 B / 500 ms  = 14_400_000 b/s (the peak).
    // Segment 2: 150_000 B / 1_500 ms = 800_000 b/s.
    publish(&w, 1, 500, 900_000);
    publish(&w, 2, 1_500, 150_000);

    let master = master(&o);
    let variant = &master.variants[0];
    // Peak: ceil(900_000 * 8 / 0.5) = 14_400_000.
    assert_eq!(variant.bandwidth, 14_400_000);
    // Mean: (900_000 + 150_000) * 8 / (0.5 + 1.5) = 8_400_000 / 2 = 4_200_000.
    // A bytes/duration swap ((900_000/1.5 + 150_000/0.5)/2) would be
    // 3_900_000; a peak-as-average mistake would be 14_400_000.
    assert_average(
        variant.average_bandwidth,
        variant.bandwidth,
        4_200_000,
        "two unequal segments",
    );
}

#[test]
fn average_bandwidth_survives_window_eviction() {
    // A window of the minimum size: the first (largest, slowest) segment is
    // evicted long before the average is read, yet it must still count.
    let (trunk, w) = trunk_with(8);
    let o = origin(&trunk, 1);
    publish(&w, 1, 2_000, 2_000_000); // 8 Mb/s over 2 s
    for seq in 2..=6 {
        publish(&w, seq, 1_000, 100_000); // 800_000 b/s over 1 s, five times
    }

    let master = master(&o);
    let variant = &master.variants[0];
    assert_eq!(variant.bandwidth, 8_000_000, "peak is segment 1");
    // All six segments: (2_000_000 + 5 * 100_000) * 8 / (2 + 5) s
    //   = 20_000_000 / 7 = 2_857_142.857.. -> 2_857_143.
    // Only the five still in the window would be 800_000; the peak is
    // 8_000_000.
    assert_average(
        variant.average_bandwidth,
        variant.bandwidth,
        2_857_143,
        "after evicting the first segment",
    );
}

#[test]
fn average_bandwidth_is_omitted_when_nothing_is_measurable() {
    let (trunk, w) = trunk_with(8);
    let o = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
        .expect("origin builds");

    let before = master(&o);
    assert_eq!(
        before.variants[0].average_bandwidth, None,
        "no segment drained yet: AVERAGE-BANDWIDTH must be absent, not invented"
    );

    // A zero-duration segment has no measurable bitrate and must not make the
    // origin claim one.
    publish(&w, 1, 0, 10_000);
    let after = master(&o);
    assert_eq!(
        after.variants[0].average_bandwidth, None,
        "a zero-duration segment alone is not measurable"
    );
    assert_eq!(
        after.variants[0].bandwidth, 5_000_000,
        "placeholder BANDWIDTH"
    );
}

#[test]
fn average_bandwidth_is_clamped_to_bandwidth() {
    // Two equal-duration segments: the mean bitrate is their arithmetic mean,
    // rounded to nearest, while `BANDWIDTH` is the peak rounded up. 1 B and
    // 2 B over 2 s each mean 0.75 B/s = 6 b/s (round: 6) against a peak of
    // 1 B/s = 8 b/s (ceil: 8).
    let (trunk, w) = trunk_with(8);
    let o = origin(&trunk, 4);
    publish(&w, 1, 2_000, 1);
    publish(&w, 2, 2_000, 2);

    let variant = master(&o).variants[0].clone();
    assert_eq!(variant.bandwidth, 8, "peak: ceil(2 B * 8 / 2 s)");
    // Mean: (1 + 2) * 8 B*bits / 4 s = 24 / 4 = 6 b/s.
    assert_eq!(variant.average_bandwidth, Some(6));
    assert!(
        variant.average_bandwidth.unwrap_or(u32::MAX) <= variant.bandwidth,
        "average must never exceed the peak BANDWIDTH"
    );
}

/// The clamp holds on the rendered line itself: `AVERAGE-BANDWIDTH` must
/// never be greater than the `BANDWIDTH` of the same `#EXT-X-STREAM-INF`.
#[test]
fn rendered_average_never_exceeds_rendered_bandwidth() {
    let (trunk, w) = trunk_with(8);
    let o = origin(&trunk, 1);
    publish(&w, 1, 3_000, 1001); // 2669.33 b/s
    publish(&w, 2, 1_000, 300); // 2400 b/s
    publish(&w, 3, 1_000, 400); // 3200 b/s
    let rendered = o.master_playlist("media.m3u8").expect("master renders");
    let line = rendered
        .lines()
        .find(|l| l.starts_with("#EXT-X-STREAM-INF:"))
        .expect("a STREAM-INF line");
    let bandwidth: u32 = line
        .split("BANDWIDTH=")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .and_then(|n| n.parse().ok())
        .expect("BANDWIDTH renders");
    if let Some(rest) = line.split("AVERAGE-BANDWIDTH=").nth(1) {
        let average: u32 = rest
            .split(',')
            .next()
            .and_then(|n| n.parse().ok())
            .expect("AVERAGE-BANDWIDTH is a decimal-integer");
        assert!(
            average <= bandwidth,
            "AVERAGE-BANDWIDTH {average} exceeds BANDWIDTH {bandwidth}: {line}"
        );
    }
}
