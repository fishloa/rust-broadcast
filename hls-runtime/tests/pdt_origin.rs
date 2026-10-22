#![cfg(feature = "std")]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::MediaPlaylist;
use bytes::Bytes;
use hls_runtime::server::{BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsRequest};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{PartEntry, SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use timed_metadata::TimeAnchor;
use transmux::SegmentMeta;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn trunk_with() -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    let writer = trunk.segment_writer().expect("writer");
    (trunk, writer)
}

fn origin(trunk: &Arc<Trunk>) -> HlsOrigin {
    HlsOrigin::builder(Arc::clone(trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .build()
        .expect("origin")
}

fn seg(writer: &SegmentWriter, seq: u32, secs: f64, discontinuous: bool) {
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(vec![seq as u8; 8]),
            SegmentSeq::new(seq),
            Duration::from_secs_f64(secs),
            Timestamp::from_nanos(0),
            SegmentMeta { discontinuous },
        ))
        .expect("publish");
}

fn part(writer: &SegmentWriter, seg_no: u32, idx: u32) {
    writer.publish_part(PartEntry::new(
        Bytes::from(vec![idx as u8; 4]),
        SegmentSeq::new(seg_no),
        media_plane::ids::PartIndex::new(idx),
        Duration::from_millis(500),
        idx == 0,
    ));
}

fn patient() -> (Timestamp, AwaitPolicy) {
    (
        Timestamp::from_nanos(1),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    )
}

fn playlist_text(origin: &HlsOrigin) -> String {
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
            body: HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected playlist, got {other:?}"),
    }
}

fn expected_pdt(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .unwrap()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

#[test]
fn anchor_and_starts_present_pdt_on_every_segment() {
    let (trunk, writer) = trunk_with();
    writer.set_time_anchor(TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 1_000_000_000_000,
    });
    let origin = origin(&trunk);
    origin.set_init(vec![0xAAu8; 8]);
    writer.note_segment_start(SegmentSeq::new(1), timed_metadata::MediaTime(0));
    writer.note_segment_start(SegmentSeq::new(2), timed_metadata::MediaTime(4_000_000));
    seg(&writer, 1, 4.0, false);
    seg(&writer, 2, 4.0, false);
    let body = playlist_text(&origin);
    let pdt1 = expected_pdt(1_000_000_000_000);
    let pdt2 = expected_pdt(1_000_000_000_000i64 + 4_000_000i64 * 1000 / 90_000);
    assert!(
        body.contains(&format!("#EXT-X-PROGRAM-DATE-TIME:{pdt1}\n")),
        "body: {body}"
    );
    assert!(
        body.contains(&format!("#EXT-X-PROGRAM-DATE-TIME:{pdt2}\n")),
        "body: {body}"
    );
}

#[test]
fn open_segment_pdt_precedes_first_part() {
    let (trunk, writer) = trunk_with();
    writer.set_time_anchor(TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 1_000_000_000_000,
    });
    let origin = origin(&trunk);
    origin.set_init(vec![0xAAu8; 8]);
    writer.note_segment_start(SegmentSeq::new(1), timed_metadata::MediaTime(0));
    writer.note_segment_start(SegmentSeq::new(2), timed_metadata::MediaTime(4_000_000));
    seg(&writer, 1, 4.0, false);
    part(&writer, 2, 0);
    let body = playlist_text(&origin);
    let pdt2 = expected_pdt(1_000_000_000_000i64 + 4_000_000i64 * 1000 / 90_000);
    let pdt_idx = body
        .find(&format!("#EXT-X-PROGRAM-DATE-TIME:{pdt2}"))
        .expect("pdt present");
    let part_idx = body.find("#EXT-X-PART:").expect("part present");
    assert!(pdt_idx < part_idx, "body: {body}");
}

#[test]
fn anchor_absent_no_pdt_anywhere() {
    let (trunk, writer) = trunk_with();
    let origin = origin(&trunk);
    origin.set_init(vec![0xAAu8; 8]);
    writer.note_segment_start(SegmentSeq::new(1), timed_metadata::MediaTime(0));
    writer.note_segment_start(SegmentSeq::new(2), timed_metadata::MediaTime(4_000_000));
    seg(&writer, 1, 4.0, false);
    seg(&writer, 2, 4.0, false);
    let body = playlist_text(&origin);
    assert!(!body.contains("PROGRAM-DATE-TIME"), "body: {body}");
}

#[test]
fn anchor_present_one_unknown_start_others_have_pdt() {
    let (trunk, writer) = trunk_with();
    writer.set_time_anchor(TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 1_000_000_000_000,
    });
    let origin = origin(&trunk);
    origin.set_init(vec![0xAAu8; 8]);
    writer.note_segment_start(SegmentSeq::new(1), timed_metadata::MediaTime(0));
    // seq 2 unknown
    seg(&writer, 1, 4.0, false);
    seg(&writer, 2, 4.0, false);
    let body = playlist_text(&origin);
    let pdt1 = expected_pdt(1_000_000_000_000);
    assert!(
        body.contains(&format!("#EXT-X-PROGRAM-DATE-TIME:{pdt1}\n")),
        "body: {body}"
    );
    // seq2's expected pdt should be absent
    let pdt2 = expected_pdt(1_000_000_000_000i64 + 4_000_000i64 * 1000 / 90_000);
    assert!(
        !body.contains(&format!("#EXT-X-PROGRAM-DATE-TIME:{pdt2}")),
        "body: {body}"
    );
}

#[test]
fn discontinuity_sequence_zero_on_fresh_live_playlist() {
    let (trunk, writer) = trunk_with();
    let origin = origin(&trunk);
    origin.set_init(vec![0xAAu8; 8]);
    seg(&writer, 1, 4.0, false);
    let body = playlist_text(&origin);
    assert!(
        body.contains("#EXT-X-DISCONTINUITY-SEQUENCE:0\n"),
        "body: {body}"
    );
}

#[test]
fn discontinuity_sequence_zero_on_iframe_playlist() {
    // use origin_fmp4 style
    let (trunk, writer) = {
        let t = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(64), nz(8), nz(64)));
        let w = t.segment_writer().expect("writer");
        (t, w)
    };
    let origin = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .build()
        .expect("origin");
    let init = test_fixtures::read("hls/cmaf-fmp4/init.mp4");
    origin.set_init(Bytes::from(init));
    for i in 0..3u32 {
        let bytes = test_fixtures::read(&format!("hls/cmaf-fmp4/index{}.m4s", i));
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(bytes),
                SegmentSeq::new(i + 1),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    let (now, policy) = patient();
    let resp = origin.resolve(
        HlsRequest::IFramePlaylist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        now,
        policy,
    );
    let body = match resp {
        EgressResponse::Ready {
            body: HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected iframe playlist, got {other:?}"),
    };
    assert!(
        body.contains("#EXT-X-DISCONTINUITY-SEQUENCE:0\n"),
        "body: {body}"
    );
}

#[test]
fn discontinuity_sequence_nonzero_after_window_slide_with_discontinuity() {
    let (trunk, writer) = trunk_with();
    // window capacity 4
    let origin = origin(&trunk);
    origin.set_init(vec![0xAAu8; 8]);
    seg(&writer, 1, 4.0, false);
    seg(&writer, 2, 4.0, true); // discontinuous
    seg(&writer, 3, 4.0, false);
    seg(&writer, 4, 4.0, false);
    seg(&writer, 5, 4.0, false);
    seg(&writer, 6, 4.0, false);
    let body = playlist_text(&origin);
    assert!(
        body.contains("#EXT-X-DISCONTINUITY-SEQUENCE:1\n"),
        "body: {body}"
    );
}

#[test]
fn parse_serialize_byte_stability_of_origin_output() {
    let (trunk, writer) = trunk_with();
    writer.set_time_anchor(TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 1_000_000_000_000,
    });
    let origin = origin(&trunk);
    origin.set_init(vec![0xAAu8; 8]);
    writer.note_segment_start(SegmentSeq::new(1), timed_metadata::MediaTime(0));
    writer.note_segment_start(SegmentSeq::new(2), timed_metadata::MediaTime(4_000_000));
    seg(&writer, 1, 4.0, false);
    seg(&writer, 2, 4.0, false);
    let body = playlist_text(&origin);
    let parsed = MediaPlaylist::parse(&body).expect("parse");
    let reserialized = parsed.to_m3u8().expect("serialize");
    assert_eq!(
        body, reserialized,
        "body:\n{body}\nreserialized:\n{reserialized}"
    );
}
