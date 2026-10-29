//! LL-HLS Playlist Delta Updates (RFC 8216bis §6.2.5.1) via `_HLS_skip`.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::MediaPlaylist;
use bytes::Bytes;
use hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsRequest, SkipDirective,
};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::EventAnchor;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use timed_metadata::{MediaTime, TimeAnchor};
use transmux::SegmentMeta;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn make_origin() -> (Arc<Trunk>, HlsOrigin, SegmentWriter) {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(16), nz(8), nz(64)));
    let writer = trunk.segment_writer().expect("writer");
    let origin = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(16))
        .low_latency(500)
        .instance(7)
        .build()
        .expect("origin");
    origin.set_init(vec![0xAAu8; 8]);
    (trunk, origin, writer)
}

fn seg(w: &SegmentWriter, seq: u32) {
    w.publish_segment(SegmentEntry::new(
        Bytes::from(vec![seq as u8; 8]),
        SegmentSeq::new(seq),
        Duration::from_secs(4),
        Timestamp::from_nanos(0),
        SegmentMeta {
            discontinuous: false,
        },
    ))
    .unwrap();
}

fn render(origin: &HlsOrigin, skip: Option<SkipDirective>) -> String {
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(0)),
    );
    match origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery {
                hls_msn: None,
                hls_part: None,
                hls_skip: skip,
                hls_primary: None,
            },
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[test]
fn skip_delta_count_and_media_sequence() {
    let (_trunk, origin, writer) = make_origin();
    for s in 1..=8 {
        seg(&writer, s);
    }
    let full = render(&origin, None);
    let parsed_full = MediaPlaylist::parse(&full).expect("parse full");
    let full_msn = parsed_full.media_sequence;

    let delta = render(&origin, Some(SkipDirective::Yes));
    let parsed = MediaPlaylist::parse(&delta).expect("parse delta");
    let skip = parsed.skip.clone().expect("EXT-X-SKIP present");
    // 8 segments * 4s = 32s, boundary 24s -> 2 skipped (32-24=8? actually 2)
    assert_eq!(
        skip.skipped_segments, 2,
        "skipped={}",
        skip.skipped_segments
    );
    assert_eq!(parsed.media_sequence, full_msn, "MEDIA-SEQUENCE unchanged");
    // re-serialise round trips
    let reser = parsed.to_m3u8().expect("reserialise");
    let reparsed = MediaPlaylist::parse(&reser).expect("reparse");
    assert_eq!(
        reparsed.skip.map(|s| s.skipped_segments),
        Some(skip.skipped_segments)
    );
}

#[test]
fn skip_delta_second_range() {
    let (_trunk, origin, writer) = make_origin();
    for s in 1..=12 {
        seg(&writer, s);
    }
    let delta = render(&origin, Some(SkipDirective::Yes));
    let parsed = MediaPlaylist::parse(&delta).expect("parse delta");
    let skip = parsed.skip.clone().expect("EXT-X-SKIP present");
    assert_eq!(
        skip.skipped_segments, 6,
        "skipped={}",
        skip.skipped_segments
    );
}

#[test]
fn skip_v2_emits_recently_removed() {
    let (_trunk, origin, writer) = make_origin();
    for s in 1..=8 {
        seg(&writer, s);
    }
    let delta = render(&origin, Some(SkipDirective::V2));
    let parsed = MediaPlaylist::parse(&delta).expect("parse delta");
    assert!(parsed.skip.is_some());
}

#[test]
fn no_can_skip_until_means_no_skip() {
    // classic origin (no low latency) does not advertise CAN-SKIP-UNTIL
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(16), nz(8), nz(64)));
    let writer = trunk.segment_writer().expect("writer");
    let origin = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(8))
        .instance(7)
        .build()
        .expect("origin");
    origin.set_init(vec![0xAAu8; 8]);
    for s in 1..=8 {
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(vec![s as u8; 8]),
                SegmentSeq::new(s),
                Duration::from_secs(4),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .unwrap();
    }
    let delta = render(&origin, Some(SkipDirective::Yes));
    let parsed = MediaPlaylist::parse(&delta).expect("parse");
    assert!(parsed.skip.is_none(), "no skip without CAN-SKIP-UNTIL");
    assert!(!delta.contains("CAN-SKIP-UNTIL"));
}

#[test]
fn combined_blocking_and_skip() {
    let (_trunk, origin, writer) = make_origin();
    for s in 1..=8 {
        seg(&writer, s);
    }
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    let resp = origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery {
                hls_msn: Some(5),
                hls_part: None,
                hls_skip: Some(SkipDirective::Yes),
                hls_primary: None,
            },
        },
        now,
        policy,
    );
    match resp {
        EgressResponse::Ready {
            body: HlsBody::Playlist(m),
            ..
        } => {
            let parsed = MediaPlaylist::parse(&m).expect("parse");
            assert!(parsed.skip.is_some());
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[test]
fn bad_skip_value_is_ignored() {
    // Engine only sees typed directives; None represents an ignored/bad value.
    let (_trunk, origin, writer) = make_origin();
    for s in 1..=8 {
        seg(&writer, s);
    }
    let delta = render(&origin, None);
    let parsed = MediaPlaylist::parse(&delta).expect("parse");
    assert!(parsed.skip.is_none());
}

#[test]
fn skip_yes_keeps_dateranges_v2_drops_them() {
    let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
    let splice_bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();

    for v2 in [false, true] {
        let (trunk, origin, writer) = make_origin();
        writer.set_time_anchor(TimeAnchor {
            pts_90k: 0,
            utc_epoch_ms: 1_000_000_000_000,
        });
        // two dateranges: one in a skipped segment (seg 1), one inside.
        let mut timeline = timed_metadata::Timeline::new();
        let ev = timeline.push_scte35(&splice_bytes).unwrap();
        let tw = trunk.writer().expect("writer");
        writer.note_segment_start(media_plane::ids::SegmentSeq::new(1), MediaTime(0));
        tw.publish_event(ev.clone(), EventAnchor::Media(MediaTime(0)));
        writer.note_segment_start(media_plane::ids::SegmentSeq::new(7), MediaTime(24_000_000));
        tw.publish_event(ev.clone(), EventAnchor::Media(MediaTime(24_000_000)));

        for s in 1..=8 {
            seg(&writer, s);
        }

        let skip = if v2 {
            Some(SkipDirective::V2)
        } else {
            Some(SkipDirective::Yes)
        };
        let rendered = render(&origin, skip);
        let dr_count = rendered.matches("#EXT-X-DATERANGE").count();
        if v2 {
            assert_eq!(
                dr_count, 1,
                "v2 should keep only the inside daterange: {rendered}"
            );
            assert!(
                rendered.contains(
                    "#EXT-X-SKIP:SKIPPED-SEGMENTS=2,RECENTLY-REMOVED-DATERANGES=\"2002\""
                ),
                "v2 skip line: {rendered}"
            );
        } else {
            assert_eq!(dr_count, 2, "YES should keep both dateranges: {rendered}");
            assert!(
                rendered.contains("#EXT-X-SKIP:SKIPPED-SEGMENTS=2\n"),
                "YES skip line: {rendered}"
            );
        }
    }
}
