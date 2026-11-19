#![cfg(feature = "std")]
//! Audit 22 finding 55 (issue #3023): the origin's Media Playlist field
//! construction — `EXT-X-GAP`, `EXT-X-DISCONTINUITY`, the LL-HLS hold-back /
//! skip-boundary multipliers, `EXT-X-RENDITION-REPORT` for sibling
//! renditions, and the Playlist Delta Update skip boundary.
//!
//! Spec: RFC 8216bis §4.4.3.8 (`HOLD-BACK` >= 3x Target Duration),
//! §6.2.5.1 (Skip Boundary >= 6x Target Duration), §4.4.4.7 (`EXT-X-GAP`),
//! §4.3.2.3 (`EXT-X-DISCONTINUITY`), §4.4.5.4 (`EXT-X-RENDITION-REPORT`).

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsOriginConfig, HlsRequest,
    RenditionSibling, SkipDirective,
};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{EventAnchor, SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use timed_metadata::{MediaTime, TimeAnchor};
use transmux::SegmentMeta;

const UTC_EPOCH_MS: i64 = 1_000_000_000_000;
const PTS_TICKS_PER_SEC: u64 = 90_000;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn trunk_with(capacity: usize) -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(64))
            .with_sparse_capacity(nz(8))
            .with_segment_capacity(nz(capacity))
            .with_event_capacity(nz(8))
            .with_part_capacity(nz(64)),
    );
    let writer = trunk.segment_writer().expect("segment writer");
    (trunk, writer)
}

fn origin_for(
    trunk: &Arc<Trunk>,
    capacity: usize,
    low_latency: bool,
    playlist_uri: &str,
) -> HlsOrigin {
    let mut config = HlsOriginConfig::new(Arc::clone(trunk), Duration::from_secs(4), nz(capacity))
        .with_instance(7)
        .with_playlist_uri(playlist_uri);
    if low_latency {
        config = config.with_low_latency(Duration::from_millis(500));
    }
    HlsOrigin::new(config).expect("origin builds")
}

fn fsmp4_origin(trunk: &Arc<Trunk>, capacity: usize) -> HlsOrigin {
    let origin = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(trunk), Duration::from_secs(4), nz(capacity))
            .with_instance(7)
            .with_low_latency(Duration::from_millis(500))
            .with_container(transmux::Container::Fmp4),
    )
    .expect("origin builds");
    origin.set_init(Bytes::from(test_fixtures::read("hls/cmaf-fmp4/init.mp4")));
    origin
}

fn publish(writer: &SegmentWriter, seq: u32, secs: u64, discontinuous: bool, bytes: &[u8]) {
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::copy_from_slice(bytes),
            SegmentSeq::new(seq),
            Duration::from_secs(secs),
            Timestamp::from_nanos(0),
            SegmentMeta { discontinuous },
        ))
        .expect("monotonic sequence");
}

fn playlist(origin: &HlsOrigin, skip: Option<SkipDirective>) -> String {
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    match origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery {
                hls_skip: skip,
                ..BlockingQuery::default()
            },
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(body),
            ..
        } => body,
        other => panic!("expected a rendered playlist, got {other:?}"),
    }
}

fn iframe_playlist(origin: &HlsOrigin) -> String {
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    match origin.resolve(
        HlsRequest::IFramePlaylist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
            media_playlist_name: "media.m3u8".to_string(),
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(body),
            ..
        } => body,
        other => panic!("expected a rendered I-frame playlist, got {other:?}"),
    }
}

#[test]
fn a_segment_published_without_bytes_is_marked_ext_x_gap() {
    let (trunk, writer) = trunk_with(8);
    let origin = origin_for(&trunk, 8, false, "media.m3u8");
    publish(&writer, 1, 4, false, &[1; 8]);
    publish(&writer, 2, 4, false, &[]);
    publish(&writer, 3, 4, false, &[3; 8]);

    let body = playlist(&origin, None);
    let lines: Vec<&str> = body.lines().collect();
    let mut gaps = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if *line == "#EXT-X-GAP" {
            gaps.push(i);
        }
    }
    assert_eq!(
        gaps.len(),
        1,
        "exactly the byte-less segment is a gap: {body}"
    );
    let gap = gaps[0];
    assert_eq!(lines[gap + 1], "#EXTINF:4,", "{body}");
    assert_eq!(lines[gap + 2], "seg-1-7-2.m4s", "{body}");
}

#[test]
fn a_mid_window_discontinuity_is_tagged_in_the_media_and_iframe_playlists() {
    let (trunk, writer) = trunk_with(8);
    let origin = fsmp4_origin(&trunk, 8);
    let segment = test_fixtures::read("hls/cmaf-fmp4/index0.m4s");
    publish(&writer, 1, 1, false, &segment);
    publish(&writer, 2, 1, true, &segment);
    publish(&writer, 3, 1, false, &segment);

    for (label, body) in [
        ("media", playlist(&origin, None)),
        ("iframe", iframe_playlist(&origin)),
    ] {
        let has = body
            .lines()
            .zip(body.lines().skip(1))
            .any(|(tag, next)| tag == "#EXT-X-DISCONTINUITY" && next != "#EXT-X-MAP");
        assert!(
            has,
            "{label} playlist must carry #EXT-X-DISCONTINUITY before the discontinuous segment: {body}"
        );
    }
}

#[test]
fn iframe_playlist_carries_the_low_latency_block_and_iframes_only() {
    let (trunk, writer) = trunk_with(8);
    let origin = fsmp4_origin(&trunk, 8);
    let segment = test_fixtures::read("hls/cmaf-fmp4/index0.m4s");
    publish(&writer, 1, 1, false, &segment);

    let body = iframe_playlist(&origin);
    assert!(body.contains("#EXT-X-I-FRAMES-ONLY\n"), "{body}");
    assert!(body.contains("#EXT-X-PART-INF:PART-TARGET=0.5\n"), "{body}");
    // RFC 8216bis §4.4.3.8 / §6.2.5.1: HOLD-BACK is 3x and CAN-SKIP-UNTIL is
    // 6x the 4 s Target Duration.
    assert!(
        body.contains("CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5,HOLD-BACK=12,CAN-SKIP-UNTIL=24"),
        "{body}"
    );
    assert!(
        body.lines()
            .find(|l| l.starts_with("#EXT-X-VERSION:"))
            .is_some_and(|l| l["#EXT-X-VERSION:".len()..]
                .parse::<u8>()
                .is_ok_and(|v| v >= 4)),
        "{body}"
    );
}

#[test]
fn rendition_reports_name_each_other_sibling_and_skip_the_origin_itself() {
    let (trunk_a, writer_a) = trunk_with(8);
    let (trunk_b, writer_b) = trunk_with(8);
    let (trunk_self, _writer_self) = trunk_with(8);
    let a = Arc::new(origin_for(&trunk_a, 8, true, "media.m3u8"));
    let b = Arc::new(origin_for(&trunk_b, 8, true, "audio/a0.m3u8"));
    let same_uri = Arc::new(origin_for(&trunk_self, 8, true, "media.m3u8"));
    publish(&writer_a, 1, 4, false, &[1; 8]);
    publish(&writer_b, 4, 4, false, &[4; 8]);

    a.set_rendition_siblings(vec![
        RenditionSibling {
            origin: Arc::downgrade(&b),
            playlist_uri: "audio/a0.m3u8".to_string(),
        },
        RenditionSibling {
            origin: Arc::downgrade(&same_uri),
            playlist_uri: "media.m3u8".to_string(),
        },
    ]);

    let body = playlist(&a, None);
    let reports: Vec<&str> = body
        .lines()
        .filter(|l| l.starts_with("#EXT-X-RENDITION-REPORT:"))
        .collect();
    assert_eq!(reports.len(), 1, "{body}");
    assert_eq!(
        reports[0], "#EXT-X-RENDITION-REPORT:URI=\"audio/a0.m3u8\",LAST-MSN=4",
        "{body}"
    );
}

#[test]
fn iframe_rendition_reports_keep_one_primary_and_each_other_sibling() {
    let (trunk_a, writer_a) = trunk_with(8);
    let (trunk_b, writer_b) = trunk_with(8);
    let (trunk_self, _writer_self) = trunk_with(8);
    let a = fsmp4_origin(&trunk_a, 8);
    let b = Arc::new(origin_for(&trunk_b, 8, true, "audio/a0.m3u8"));
    let same_uri = Arc::new(origin_for(&trunk_self, 8, true, "media.m3u8"));
    let segment = test_fixtures::read("hls/cmaf-fmp4/index0.m4s");
    publish(&writer_a, 1, 1, false, &segment);
    publish(&writer_b, 4, 4, false, &[4; 8]);

    a.set_rendition_siblings(vec![
        RenditionSibling {
            origin: Arc::downgrade(&b),
            playlist_uri: "audio/a0.m3u8".to_string(),
        },
        RenditionSibling {
            origin: Arc::downgrade(&same_uri),
            playlist_uri: "media.m3u8".to_string(),
        },
    ]);

    let body = iframe_playlist(&a);
    let media_refs = body
        .lines()
        .filter(|l| *l == "#EXT-X-RENDITION-REPORT:URI=\"media.m3u8\",LAST-MSN=1")
        .count();
    assert_eq!(
        media_refs, 1,
        "the origin never reports itself a second time: {body}"
    );
    assert!(
        body.contains("#EXT-X-RENDITION-REPORT:URI=\"audio/a0.m3u8\",LAST-MSN=4"),
        "{body}"
    );
}

/// RFC 8216bis §6.2.5.1: seven 4 s segments are 28 s; the first one's
/// distance from the end (28 s) exceeds the 24 s Skip Boundary, the second's
/// (24 s) does not — exactly one segment is omitted.
#[test]
fn the_skip_boundary_omits_exactly_one_segment() {
    let (trunk, writer) = trunk_with(8);
    let origin = origin_for(&trunk, 8, true, "media.m3u8");
    for seq in 1..=7 {
        publish(&writer, seq, 4, false, &[u8::try_from(seq).unwrap(); 8]);
    }
    let body = playlist(&origin, Some(SkipDirective::Yes));
    assert!(body.contains("#EXT-X-SKIP:SKIPPED-SEGMENTS=1\n"), "{body}");
    assert!(!body.contains("-1.m4s"), "{body}");
    assert!(body.contains("-2.m4s"), "{body}");
}

/// RFC 8216bis §6.2.5.1: `_HLS_skip=v2` also omits the `EXT-X-DATERANGE`
/// tags of skipped segments — but a tag on the first *kept* segment (its
/// index is exactly the omitted count) is not skipped.
#[test]
fn v2_keeps_the_daterange_on_the_first_kept_segment() {
    let (trunk, writer) = trunk_with(8);
    let origin = origin_for(&trunk, 8, true, "media.m3u8");
    let splice = test_fixtures::read("scte35/splice-insert-2002.bin");
    writer.set_time_anchor(TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: UTC_EPOCH_MS,
    });
    let mut timeline = timed_metadata::Timeline::default();
    let event = timeline.push_scte35(&splice).expect("a valid SCTE-35 cue");
    assert_eq!(event.id, Some(2002), "the fixture's splice_event_id");

    let media = trunk.writer().expect("trunk writer");
    writer.note_segment_start(SegmentSeq::new(1), MediaTime(0));
    media.publish_event(event.clone(), EventAnchor::Media(MediaTime(0)));
    writer.note_segment_start(SegmentSeq::new(2), MediaTime(4 * PTS_TICKS_PER_SEC));
    media.publish_event(
        event.clone(),
        EventAnchor::Media(MediaTime(4 * PTS_TICKS_PER_SEC)),
    );
    for seq in 1..=7 {
        publish(&writer, seq, 4, false, &[u8::try_from(seq).unwrap(); 8]);
    }

    let body = playlist(&origin, Some(SkipDirective::V2));
    assert!(
        body.contains("RECENTLY-REMOVED-DATERANGES=\"2002\""),
        "only the skipped segment's daterange is removed: {body}"
    );
    assert_eq!(
        body.matches("#EXT-X-DATERANGE").count(),
        1,
        "the first kept segment's daterange stays: {body}"
    );
}

fn publish_part(writer: &SegmentWriter, seq: u32, idx: u32) {
    writer.publish_part(media_plane::trunk::PartEntry::new(
        Bytes::from(vec![u8::try_from(idx).unwrap(); 4]),
        SegmentSeq::new(seq),
        media_plane::ids::PartIndex::new(idx),
        Duration::from_millis(500),
        idx == 0,
    ));
}

fn playlist_query(origin: &HlsOrigin, msn: u64, part: Option<u32>) -> EgressResponse<HlsBody> {
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery {
                hls_msn: Some(msn),
                hls_part: part,
                ..BlockingQuery::default()
            },
        },
        now,
        policy,
    )
}

/// RFC 8216bis §6.2.5.2: the Advance Part Limit is measured from the last
/// Partial Segment in the Playlist. Three resident parts make the limit
/// `3 - 1 + 6 = 8`; part 8 parks, part 9 is a Bad Request.
#[test]
fn the_advance_part_limit_counts_the_resident_parts_of_the_live_edge() {
    let (trunk, writer) = trunk_with(8);
    let origin = origin_for(&trunk, 8, true, "media.m3u8");
    publish_part(&writer, 1, 0);
    publish_part(&writer, 1, 1);
    publish_part(&writer, 1, 2);

    assert!(
        matches!(
            playlist_query(&origin, 1, Some(8)),
            EgressResponse::Await { .. }
        ),
        "part 8 is within the Advance Part Limit, so it parks"
    );
    assert!(matches!(
        playlist_query(&origin, 1, Some(9)),
        EgressResponse::BadRequest { .. }
    ));
}

/// The I-frame playlist's `ABUSE_MSN_FUTURE_BOUND` is inclusive: one
/// segment closed means a request for MSN 4 (`2 + 2`) still waits, MSN 5 is
/// a Bad Request.
#[test]
fn iframe_playlist_accepts_a_reload_at_the_abuse_bound() {
    let (trunk, writer) = trunk_with(8);
    let origin = fsmp4_origin(&trunk, 8);
    publish(
        &writer,
        1,
        1,
        false,
        &test_fixtures::read("hls/cmaf-fmp4/index0.m4s"),
    );

    let reload = |msn: u64| {
        let (now, policy) = (
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
        );
        origin.resolve(
            HlsRequest::IFramePlaylist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery {
                    hls_msn: Some(msn),
                    ..BlockingQuery::default()
                },
                media_playlist_name: "media.m3u8".to_string(),
            },
            now,
            policy,
        )
    };

    assert!(
        matches!(reload(4), EgressResponse::Await { .. }),
        "MSN 4 is the last one within the future bound"
    );
    assert!(matches!(reload(5), EgressResponse::BadRequest { .. }));
}
