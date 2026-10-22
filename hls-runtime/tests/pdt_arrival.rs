#![cfg(feature = "std")]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use broadcast_common::Timestamp;
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

fn origin_arrival(trunk: &Arc<Trunk>, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> HlsOrigin {
    HlsOrigin::builder(Arc::clone(trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .program_date_time_arrival(clock)
        .build()
        .expect("origin")
}

fn origin_plain(trunk: &Arc<Trunk>) -> HlsOrigin {
    HlsOrigin::builder(Arc::clone(trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .build()
        .expect("origin")
}

fn seg(writer: &SegmentWriter, seq: u32, secs: f64, start_ns: u64, discontinuous: bool) {
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(vec![seq as u8; 8]),
            SegmentSeq::new(seq),
            Duration::from_secs_f64(secs),
            Timestamp::from_nanos(start_ns),
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

fn iframe_playlist_text(origin: &HlsOrigin) -> String {
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
            body: HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected iframe playlist, got {other:?}"),
    }
}

fn expected_pdt(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .unwrap()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

const NOW_MS: i64 = 1_000_000_000_000;

#[test]
fn first_segment_pdt_equals_clock_minus_duration() {
    let (trunk, writer) = trunk_with();
    let clock = Arc::new(|| NOW_MS) as Arc<dyn Fn() -> i64 + Send + Sync>;
    let origin = origin_arrival(&trunk, clock);
    origin.set_init(vec![0xAAu8; 8]);
    seg(&writer, 1, 4.0, 0, false);
    let body = playlist_text(&origin);
    let pdt = expected_pdt(NOW_MS - 4_000);
    assert!(
        body.contains(&format!("#EXT-X-PROGRAM-DATE-TIME:{pdt}\n")),
        "body: {body}"
    );
}

#[test]
fn later_segments_advance_by_timeline_delta() {
    let (trunk, writer) = trunk_with();
    let clock = Arc::new(|| NOW_MS) as Arc<dyn Fn() -> i64 + Send + Sync>;
    let origin = origin_arrival(&trunk, clock);
    origin.set_init(vec![0xAAu8; 8]);
    seg(&writer, 1, 4.0, 0, false);
    seg(&writer, 2, 4.0, 4_000_000_000, false);
    let body = playlist_text(&origin);
    let pdt1 = expected_pdt(NOW_MS - 4_000);
    let pdt2 = expected_pdt(NOW_MS);
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
fn pdt_stays_continuous_across_discontinuity() {
    let (trunk, writer) = trunk_with();
    let clock = Arc::new(|| NOW_MS) as Arc<dyn Fn() -> i64 + Send + Sync>;
    let origin = origin_arrival(&trunk, clock);
    origin.set_init(vec![0xAAu8; 8]);
    seg(&writer, 1, 4.0, 0, false);
    seg(&writer, 2, 4.0, 4_000_000_000, true);
    let body = playlist_text(&origin);
    // segment 2 pdt == segment 1 pdt + segment 1 duration
    let pdt1 = expected_pdt(NOW_MS - 4_000);
    let pdt2 = expected_pdt(NOW_MS);
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
fn open_segment_pdt_equals_last_closed_plus_duration() {
    let (trunk, writer) = trunk_with();
    let clock = Arc::new(|| NOW_MS) as Arc<dyn Fn() -> i64 + Send + Sync>;
    let origin = origin_arrival(&trunk, clock);
    origin.set_init(vec![0xAAu8; 8]);
    seg(&writer, 1, 4.0, 0, false);
    part(&writer, 2, 0);
    let body = playlist_text(&origin);
    let open_pdt = expected_pdt(NOW_MS);
    assert!(
        body.contains(&format!("#EXT-X-PROGRAM-DATE-TIME:{open_pdt}")),
        "body: {body}"
    );
    let pdt_idx = body
        .find(&format!("#EXT-X-PROGRAM-DATE-TIME:{open_pdt}"))
        .expect("pdt");
    let part_idx = body.find("#EXT-X-PART:").expect("part");
    assert!(pdt_idx < part_idx, "body: {body}");
}

#[test]
fn explicit_anchor_wins_over_arrival() {
    let (trunk, writer) = trunk_with();
    writer.set_time_anchor(TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 9_000_000,
    });
    let clock = Arc::new(|| NOW_MS) as Arc<dyn Fn() -> i64 + Send + Sync>;
    let origin = origin_arrival(&trunk, clock);
    origin.set_init(vec![0xAAu8; 8]);
    writer.note_segment_start(SegmentSeq::new(1), timed_metadata::MediaTime(0));
    seg(&writer, 1, 4.0, 0, false);
    let body = playlist_text(&origin);
    assert!(
        body.contains("#EXT-X-PROGRAM-DATE-TIME:1970-01-01T02:30:00.000Z"),
        "body: {body}"
    );
    assert!(
        !body.contains(&expected_pdt(NOW_MS - 4_000)),
        "body: {body}"
    );
}

#[test]
fn no_clock_no_anchor_no_pdt() {
    let (trunk, writer) = trunk_with();
    let origin = origin_plain(&trunk);
    origin.set_init(vec![0xAAu8; 8]);
    seg(&writer, 1, 4.0, 0, false);
    seg(&writer, 2, 4.0, 4_000_000_000, false);
    let body = playlist_text(&origin);
    assert!(!body.contains("PROGRAM-DATE-TIME"), "body: {body}");
}

#[test]
fn clock_called_exactly_once() {
    let (trunk, writer) = trunk_with();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();
    let clock = Arc::new(move || {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        NOW_MS
    }) as Arc<dyn Fn() -> i64 + Send + Sync>;
    let origin = origin_arrival(&trunk, clock);
    origin.set_init(vec![0xAAu8; 8]);
    seg(&writer, 1, 4.0, 0, false);
    seg(&writer, 2, 4.0, 4_000_000_000, false);
    let _ = playlist_text(&origin);
    let _ = playlist_text(&origin);
    let _ = playlist_text(&origin);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "clock sampled more than once"
    );
}

#[test]
fn iframe_pdt_equals_media_pdt() {
    // fMP4 fixture setup mirroring iframe_playlist.rs.
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(64), nz(8), nz(64)));
    let writer = trunk.segment_writer().expect("writer");
    let clock = Arc::new(|| NOW_MS) as Arc<dyn Fn() -> i64 + Send + Sync>;
    let origin = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .program_date_time_arrival(clock)
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
                Timestamp::from_nanos(i as u64 * 1_000_000_000),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    let media = playlist_text(&origin);
    let iframe = iframe_playlist_text(&origin);
    let media_pdts: Vec<&str> = media
        .lines()
        .filter(|l| l.starts_with("#EXT-X-PROGRAM-DATE-TIME"))
        .collect();
    let iframe_pdts: Vec<&str> = iframe
        .lines()
        .filter(|l| l.starts_with("#EXT-X-PROGRAM-DATE-TIME"))
        .collect();
    assert_eq!(
        media_pdts, iframe_pdts,
        "media:\n{media}\niframe:\n{iframe}"
    );
    assert!(!media_pdts.is_empty());
}

#[test]
fn out_of_range_pdt_omitted_without_panic() {
    let (trunk, writer) = trunk_with();
    let clock = Arc::new(|| i64::MIN / 4) as Arc<dyn Fn() -> i64 + Send + Sync>;
    let origin = origin_arrival(&trunk, clock);
    origin.set_init(vec![0xAAu8; 8]);
    seg(&writer, 1, 4.0, 0, false);
    let body = playlist_text(&origin);
    assert!(!body.contains("PROGRAM-DATE-TIME"), "body: {body}");
}
