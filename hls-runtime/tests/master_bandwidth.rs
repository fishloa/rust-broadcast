#![cfg(feature = "std")]
//! BANDWIDTH ratchet + declared bandwidth for the sans-IO [`HlsOrigin`]'s
//! master playlist (issue #1426 part A).

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use hls_runtime::server::{HlsMasterError, HlsOrigin, HlsOriginBuildError};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::SegmentMeta;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn trunk_with(segment_cap: usize, part_cap: usize) -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(TrunkConfig::new(
        nz(64),
        nz(8),
        nz(segment_cap),
        nz(8),
        nz(part_cap),
    ));
    let writer = trunk.segment_writer().expect("first segment writer");
    (trunk, writer)
}

fn origin_declared(trunk: &Arc<Trunk>, declared: u64) -> HlsOrigin {
    HlsOrigin::builder(Arc::clone(trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(declared)
        .build()
        .expect("origin builds")
}

fn origin_undeclared(trunk: &Arc<Trunk>) -> HlsOrigin {
    HlsOrigin::builder(Arc::clone(trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .build()
        .expect("origin builds")
}

fn publish_seg_bytes(w: &SegmentWriter, seq: u32, secs: u64, bytes: usize) {
    w.publish_segment(SegmentEntry::new(
        Bytes::from(vec![0u8; bytes]),
        SegmentSeq::new(seq),
        Duration::from_secs(secs),
        broadcast_common::Timestamp::from_nanos(u64::from(seq) * 1_000_000_000),
        SegmentMeta {
            discontinuous: false,
        },
    ))
    .expect("monotonic sequence number");
}

#[test]
fn unknown_bandwidth_is_error_not_placeholder() {
    let (trunk, _w) = trunk_with(8, 64);
    let o = origin_undeclared(&trunk);
    assert_eq!(
        o.master_playlist("media.m3u8"),
        Err(HlsMasterError::BandwidthUnknown)
    );
}

#[test]
fn declared_bandwidth_used_before_any_segment() {
    let (trunk, _w) = trunk_with(8, 64);
    let o = origin_declared(&trunk, 12345);
    let m = o.master_playlist("media.m3u8").expect("master renders");
    assert!(m.contains("BANDWIDTH=12345"));
}

#[test]
fn measured_above_declared_wins() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin_declared(&trunk, 1000);
    publish_seg_bytes(&w, 1, 1, 1_500_000);
    let m = o.master_playlist("media.m3u8").expect("master renders");
    assert!(m.contains("BANDWIDTH=12000000"));
}

#[test]
fn measured_below_declared_does_not_lower() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin_declared(&trunk, 5_000_000);
    publish_seg_bytes(&w, 1, 2, 1000);
    let m = o.master_playlist("media.m3u8").expect("master renders");
    assert!(m.contains("BANDWIDTH=5000000"));
}

#[test]
fn bandwidth_never_decreases_across_calls() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin_undeclared(&trunk);
    publish_seg_bytes(&w, 1, 1, 1_500_000);
    let first = o.master_playlist("media.m3u8").expect("master renders");
    publish_seg_bytes(&w, 2, 2, 100);
    publish_seg_bytes(&w, 3, 3, 10);
    let second = o.master_playlist("media.m3u8").expect("master renders");
    // BANDWIDTH must not drop below the first measured peak.
    let bw = |s: &str| -> String {
        s.split("BANDWIDTH=")
            .nth(1)
            .unwrap()
            .split(',')
            .next()
            .unwrap()
            .split('\n')
            .next()
            .unwrap()
            .to_string()
    };
    assert!(bw(&second) >= bw(&first));
    assert_eq!(bw(&first), "12000000");
}

#[test]
fn declared_zero_or_over_u32_rejected_at_build() {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    match HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(0)
        .build()
    {
        Err(HlsOriginBuildError::InvalidDeclaredBandwidth) => {}
        _other => panic!("expected InvalidDeclaredBandwidth"),
    }
    match HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(u64::from(u32::MAX) + 1)
        .build()
    {
        Err(HlsOriginBuildError::InvalidDeclaredBandwidth) => {}
        _other => panic!("expected InvalidDeclaredBandwidth"),
    }
}

#[test]
fn average_bandwidth_still_from_totals() {
    let (trunk, w) = trunk_with(8, 64);
    let o = origin_declared(&trunk, 5_000_000);
    publish_seg_bytes(&w, 1, 1, 1_500_000);
    publish_seg_bytes(&w, 2, 2, 500_000);
    let m = o.master_playlist("media.m3u8").expect("master renders");
    assert!(m.contains("AVERAGE-BANDWIDTH="));
}
