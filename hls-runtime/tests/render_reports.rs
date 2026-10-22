#![allow(clippy::vec_init_then_push)]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::MediaPlaylist;
use bytes::Bytes;
use hls_runtime::server::{BlockingQuery, Container, DEFAULT_TRACK_ID, HlsOrigin, HlsRequest};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::ids::{PartIndex, SegmentSeq};
use media_plane::trunk::{PartEntry, SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::SegmentMeta;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn trunk_with() -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(64), nz(8), nz(64)));
    let writer = trunk.segment_writer().expect("writer");
    (trunk, writer)
}

fn origin_fmp4() -> (Arc<Trunk>, HlsOrigin, SegmentWriter) {
    let (trunk, writer) = trunk_with();
    let origin = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .build()
        .expect("origin");
    let init = std::fs::read("tests/fixtures/cmaf-fmp4/init.mp4").expect("init");
    origin.set_init(Bytes::from(init));
    (trunk, origin, writer)
}

fn origin_no_iframe() -> (Arc<Trunk>, HlsOrigin, SegmentWriter) {
    let (trunk, writer) = trunk_with();
    let origin = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .container(Container::Fmp4)
        .build()
        .expect("origin");
    (trunk, origin, writer)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!("tests/fixtures/cmaf-fmp4/{name}")).expect("fixture")
}

fn patient() -> (Timestamp, AwaitPolicy) {
    (
        Timestamp::from_nanos(1),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    )
}

fn media_text(origin: &HlsOrigin) -> String {
    let (now, policy) = patient();
    match origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected media playlist, got {other:?}"),
    }
}

fn iframe_text(origin: &HlsOrigin) -> String {
    let (now, policy) = patient();
    match origin.resolve(
        HlsRequest::IFramePlaylist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected iframe playlist, got {other:?}"),
    }
}

#[test]
fn media_reports_iframe_and_part_hold_back_floor() {
    let (_trunk, origin, writer) = origin_fmp4();
    for i in 1..=4u32 {
        writer.publish_part(PartEntry::new(
            Bytes::from(vec![0u8; 4]),
            SegmentSeq::new(i),
            PartIndex::new(0),
            Duration::from_millis(500),
            true,
        ));
        writer.publish_part(PartEntry::new(
            Bytes::from(vec![0u8; 4]),
            SegmentSeq::new(i),
            PartIndex::new(1),
            Duration::from_millis(500),
            false,
        ));
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(fixture_bytes("index0.m4s")),
                SegmentSeq::new(i),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    // open parts for the next segment
    writer.publish_part(PartEntry::new(
        Bytes::from(vec![0u8; 4]),
        SegmentSeq::new(5),
        PartIndex::new(0),
        Duration::from_millis(500),
        true,
    ));

    let mt = media_text(&origin);
    assert!(mt.contains("#EXT-X-SERVER-CONTROL:"), "media: {mt}");
    assert!(mt.contains("PART-HOLD-BACK=1.5"), "media: {mt}");
    let pm = MediaPlaylist::parse(&mt).expect("parse media");
    assert_eq!(pm.rendition_reports.len(), 1);
    assert_eq!(pm.rendition_reports[0].uri, "iframe.m3u8");
    assert!(pm.rendition_reports[0].last_part.is_none());

    let it = iframe_text(&origin);
    let pi = MediaPlaylist::parse(&it).expect("parse iframe");
    assert_eq!(pi.rendition_reports.len(), 1);
    assert_eq!(pi.rendition_reports[0].uri, "media.m3u8");
    // LAST-PART should be present because of open parts
    assert!(pi.rendition_reports[0].last_part.is_some());
}

#[test]
fn no_report_without_iframe_availability() {
    let (_trunk, origin, writer) = origin_no_iframe();
    for i in 1..=3u32 {
        writer.publish_part(PartEntry::new(
            Bytes::from(vec![0u8; 4]),
            SegmentSeq::new(i),
            PartIndex::new(0),
            Duration::from_millis(500),
            true,
        ));
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(vec![0u8; 8]),
                SegmentSeq::new(i),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    let mt = media_text(&origin);
    assert!(!mt.contains("EXT-X-RENDITION-REPORT"), "media: {mt}");
    // iframe playlist should not be available
    let (now, policy) = patient();
    let resp = origin.resolve(
        HlsRequest::IFramePlaylist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        now,
        policy,
    );
    assert!(
        !matches!(resp, EgressResponse::Ready { .. }),
        "expected unavailable, got {resp:?}"
    );
}

#[test]
fn last_msn_values_match_live_window() {
    let (_trunk, origin, writer) = origin_fmp4();
    for i in 1..=3u32 {
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(fixture_bytes("index0.m4s")),
                SegmentSeq::new(i),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    let mt = media_text(&origin);
    let pm = MediaPlaylist::parse(&mt).expect("parse media");
    // iframe last msn = 3
    assert_eq!(pm.rendition_reports[0].last_msn, 3);

    let it = iframe_text(&origin);
    let pi = MediaPlaylist::parse(&it).expect("parse iframe");
    // media last msn = 3 (no open parts)
    assert_eq!(pi.rendition_reports[0].last_msn, 3);
    assert!(pi.rendition_reports[0].last_part.is_none());
}
