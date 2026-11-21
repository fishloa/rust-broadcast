//! Read planning and the torn-line rule (DESIGN §3.6, §3.7).

use std::num::NonZeroU64;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_common::range::ByteSpan;
use dvr_runtime::read::{
    ArchiveScan, InitRef, LiveSegment, PlaylistError, UnsupportedTag, checked_range, merge,
    parse_period_playlist, window,
};
use dvr_runtime::{ArchiveContainer, PeriodNum};
use media_plane::ids::SegmentSeq;

use crate::support::{present_event, segment_at};
use dvb_si::epg::PresentEventTracker;

fn archived(
    seq: u32,
    start_ns: u64,
    secs: u64,
    discontinuous: bool,
    period: u32,
) -> dvr_runtime::read::ArchivedSegment {
    dvr_runtime::read::ArchivedSegment::new(
        SegmentSeq::new(seq),
        Timestamp::from_nanos(start_ns),
        Duration::from_secs(secs),
        PeriodNum::new(period),
        ByteSpan::new(u64::from(seq) * 10, NonZeroU64::new(10).expect("nonzero")),
    )
    .with_discontinuous(discontinuous)
    .with_init(Some(ByteSpan::new(0, NonZeroU64::new(4).expect("nonzero"))))
}

fn catchup(
    seq: u32,
    start_ns: u64,
    secs: u64,
    discontinuous: bool,
) -> dvr_runtime::read::CatchupSegment<u8> {
    dvr_runtime::read::CatchupSegment::new(
        SegmentSeq::new(seq),
        Timestamp::from_nanos(start_ns),
        Duration::from_secs(secs),
    )
    .with_discontinuous(discontinuous)
}

fn live(seq: u32, start_ns: u64, secs: u64, discontinuous: bool) -> LiveSegment<u8> {
    LiveSegment::new(
        SegmentSeq::new(seq),
        Timestamp::from_nanos(start_ns),
        Duration::from_secs(secs),
        1,
    )
    .with_discontinuous(discontinuous)
}

#[test]
fn merge_excludes_archived_segments_from_the_live_tail() {
    let archived = [
        archived(1, 0, 2, false, 0),
        archived(2, 2_000_000_000, 2, false, 0),
        archived(3, 4_000_000_000, 2, false, 0),
    ];
    let live = [
        live(1, 0, 2, false),
        live(2, 2_000_000_000, 2, false),
        live(3, 4_000_000_000, 2, false),
        live(4, 6_000_000_000, 2, false),
    ];
    let combined = merge(&archived, &live);
    let seqs: Vec<u32> = combined.iter().map(|s| s.seq.value()).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4]);
    assert_eq!(
        combined[0].init,
        Some(InitRef::Archived {
            period: PeriodNum::new(0),
            span: ByteSpan::new(0, NonZeroU64::new(4).expect("nonzero"))
        })
    );
    assert_eq!(combined[3].init, Some(InitRef::Live(1)));
}

#[test]
fn merge_marks_a_gap_between_archive_and_live_tail() {
    let archived = [archived(2, 2_000_000_000, 2, false, 0)];
    let gapped = merge(
        &archived,
        &[
            live(7, 12_000_000_000, 2, false),
            live(8, 14_000_000_000, 2, false),
        ],
    );
    assert!(gapped[1].discontinuous, "the tail skips 3..=6");
    assert!(!gapped[2].discontinuous);
    let contiguous = merge(&archived, &[live(3, 4_000_000_000, 2, false)]);
    assert!(!contiguous[1].discontinuous);
}

#[test]
fn merge_empty_archive_uses_every_live_segment() {
    let combined = merge(
        &[],
        &[live(5, 0, 1, false), live(6, 1_000_000_000, 1, true)],
    );
    let seqs: Vec<u32> = combined.iter().map(|s| s.seq.value()).collect();
    assert_eq!(seqs, vec![5, 6]);
    assert!(combined[1].discontinuous);
}

#[test]
fn window_keeps_only_the_trailing_seconds() {
    let combined = [
        catchup(1, 0, 10, false),
        catchup(2, 10_000_000_000, 10, false),
        catchup(3, 20_000_000_000, 10, false),
    ];
    let windowed = window(&combined, Some(Duration::from_secs(5)));
    let seqs: Vec<u32> = windowed.iter().map(|s| s.seq.value()).collect();
    assert_eq!(seqs, vec![3]);
}

#[test]
fn window_is_correct_across_a_reconnect_whose_pts_restarted() {
    let segment = |seq: u32, start_s: u64, discontinuous: bool| {
        catchup(seq, start_s * 1_000_000_000, 4, discontinuous)
    };
    let combined = [
        segment(1, 1000, false),
        segment(2, 1004, false),
        segment(3, 1008, false),
        segment(4, 1012, false),
        segment(5, 1016, false),
        segment(7, 0, true),
        segment(8, 4, false),
        segment(9, 8, false),
    ];
    let seqs = |w| -> Vec<u32> {
        window(&combined, Some(Duration::from_secs(w)))
            .iter()
            .map(|s| s.seq.value())
            .collect()
    };
    assert_eq!(seqs(8), vec![7, 8, 9]);
    assert_eq!(seqs(12), vec![5, 7, 8, 9]);
    assert_eq!(seqs(4), vec![8, 9]);
    assert_eq!(seqs(1), vec![9]);
    assert_eq!(seqs(10_000), vec![1, 2, 3, 4, 5, 7, 8, 9]);
    assert!(window::<u8>(&[], Some(Duration::from_secs(5))).is_empty());
}

#[test]
fn window_none_or_zero_returns_everything() {
    let combined = [catchup(1, 0, 2, false)];
    assert_eq!(window(&combined, None).len(), 1);
    assert_eq!(window(&combined, Some(Duration::ZERO)).len(), 1);
}

#[test]
fn a_playlist_without_the_archive_marker_is_not_an_archive() {
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2,\np0.m4s\n";
    assert!(matches!(
        parse_period_playlist(PeriodNum::new(0), ArchiveContainer::Fmp4, text.as_bytes()),
        Err(PlaylistError::NotAnArchive)
    ));
}

#[test]
fn a_marker_for_another_period_or_container_is_rejected() {
    let golden = test_fixtures::read_to_string("golden/dvr-runtime/p4.m3u8");
    assert!(matches!(
        parse_period_playlist(PeriodNum::new(5), ArchiveContainer::Fmp4, golden.as_bytes()),
        Err(PlaylistError::PeriodMismatch { tag: 4, file: 5 })
    ));
    assert!(matches!(
        parse_period_playlist(PeriodNum::new(4), ArchiveContainer::Ts, golden.as_bytes()),
        Err(PlaylistError::ContainerMismatch)
    ));
    let other_version = golden.replace("VERSION=1", "VERSION=2");
    assert!(matches!(
        parse_period_playlist(
            PeriodNum::new(4),
            ArchiveContainer::Fmp4,
            other_version.as_bytes()
        ),
        Err(PlaylistError::UnsupportedVersion { found: 2 })
    ));
}

#[test]
fn a_sequence_mismatch_and_a_foreign_uri_are_typed() {
    let golden = test_fixtures::read_to_string("golden/dvr-runtime/p4.m3u8");
    let bad_seq = golden.replace("SEQ=121", "SEQ=999");
    assert!(matches!(
        parse_period_playlist(
            PeriodNum::new(4),
            ArchiveContainer::Fmp4,
            bad_seq.as_bytes()
        ),
        Err(PlaylistError::SequenceMismatch { index: 1 })
    ));
    let foreign = golden.replace("p4.m4s\n", "p9.m4s\n");
    assert!(matches!(
        parse_period_playlist(
            PeriodNum::new(4),
            ArchiveContainer::Fmp4,
            foreign.as_bytes()
        ),
        Err(PlaylistError::ForeignUri { index: 0 })
    ));
    let no_timing = golden.replace("#EXT-X-RUSTBROADCAST-TIMING:SEQ=120,DTS=240000000000,DUR=2000000000,TIMESCALE=1000000000\n", "");
    assert!(matches!(
        parse_period_playlist(
            PeriodNum::new(4),
            ArchiveContainer::Fmp4,
            no_timing.as_bytes()
        ),
        Err(PlaylistError::MissingTiming { index: 0 })
    ));
    let gap = golden.replace(
        "#EXT-X-RUSTBROADCAST-TIMING:SEQ=121",
        "#EXT-X-GAP\n#EXT-X-RUSTBROADCAST-TIMING:SEQ=121",
    );
    assert!(matches!(
        parse_period_playlist(PeriodNum::new(4), ArchiveContainer::Fmp4, gap.as_bytes()),
        Err(PlaylistError::Unsupported {
            index: 1,
            tag: UnsupportedTag::Gap
        })
    ));
}

#[test]
fn torn_tail_every_byte_keeps_exactly_the_complete_records() {
    let golden = test_fixtures::read_to_string("golden/dvr-runtime/p4.m3u8");
    let header_end = golden.find("#EXT-X-DATERANGE:").expect("first record");
    let uris: Vec<usize> = golden
        .match_indices("p4.m4s\n")
        .map(|(at, _)| at + "p4.m4s\n".len())
        .collect();
    for cut in header_end..=golden.len() {
        let mut bytes = golden.as_bytes()[..cut].to_vec();
        bytes.extend_from_slice(&[0, 0, 0]);
        let parsed = parse_period_playlist(PeriodNum::new(4), ArchiveContainer::Fmp4, &bytes)
            .unwrap_or_else(|e| panic!("cut at {cut}: {e}"));
        let complete = uris.iter().filter(|end| **end <= cut).count();
        assert_eq!(
            parsed.segments.len(),
            complete,
            "cut at {cut}: expected {complete} complete records"
        );
        assert!(parsed.torn_bytes >= 3, "cut at {cut}: the NUL tail is torn");
        assert!(parsed.torn_records <= 1);
        for (index, segment) in parsed.segments.iter().enumerate() {
            assert_eq!(segment.seq, SegmentSeq::new(120 + index as u32));
        }
    }
}

#[test]
fn checked_range_rejects_a_span_the_file_cannot_back() {
    let good = ByteSpan::new(0, NonZeroU64::new(10).expect("nonzero"));
    assert_eq!(checked_range(good, 10).expect("exact"), good);
    assert!(matches!(
        checked_range(ByteSpan::new(1, NonZeroU64::new(10).expect("nonzero")), 10),
        Err(dvr_runtime::read::ReadError::RangeBeyondFile { file_len: 10, .. })
    ));
    let overflow = ByteSpan::new(u64::MAX, NonZeroU64::new(2).expect("nonzero"));
    assert!(matches!(
        checked_range(overflow, u64::MAX),
        Err(dvr_runtime::read::ReadError::RangeBeyondFile { .. })
    ));
}

#[test]
fn a_period_playlist_exposes_its_programmes_and_wall_times() {
    let golden = test_fixtures::read_to_string("golden/dvr-runtime/p4.m3u8");
    let parsed =
        parse_period_playlist(PeriodNum::new(4), ArchiveContainer::Fmp4, golden.as_bytes())
            .expect("parses");
    assert_eq!(
        parsed.segments[0].wall,
        Some(jiff::Timestamp::from_millisecond(1_716_055_202_000).expect("ts"))
    );
    assert_eq!(
        parsed.programmes[0].scheduled_start,
        Some(jiff::Timestamp::from_millisecond(1_716_055_200_000).expect("ts"))
    );
    let mut tracker = PresentEventTracker::new(1537);
    let change = present_event(
        &mut tracker,
        1537,
        30807,
        "Die \"Drei\" Frage",
        crate::support::ts_at(0),
        Duration::from_secs(3600),
    );
    let _ = change;
    let _ = ArchiveScan::default();
    let _ = segment_at(0, 1, 0, false);
}

fn ts_playlist(records: &str) -> String {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:EVENT\n#EXT-X-RUSTBROADCAST-ARCHIVE:VERSION=1,CONTAINER=TS,PERIOD=0\n{records}"
    )
}

fn ts_record(seq: u32, offset: u64) -> String {
    format!(
        "#EXT-X-RUSTBROADCAST-TIMING:SEQ={seq},DTS={},DUR=2000000000,TIMESCALE=1000000000\n#EXTINF:2,\n#EXT-X-BYTERANGE:100@{offset}\np0.ts\n",
        u64::from(seq) * 2_000_000_000
    )
}

#[test]
fn a_record_after_endlist_is_rejected() {
    let text = ts_playlist(&format!(
        "{}#EXT-X-ENDLIST\n{}",
        ts_record(0, 0),
        ts_record(1, 100)
    ));
    assert!(matches!(
        parse_period_playlist(PeriodNum::new(0), ArchiveContainer::Ts, text.as_bytes()),
        Err(PlaylistError::RecordAfterEndlist)
    ));
}

#[test]
fn two_endlists_are_rejected() {
    let text = ts_playlist(&format!(
        "{}#EXT-X-ENDLIST\n#EXT-X-ENDLIST\n",
        ts_record(0, 0)
    ));
    assert!(matches!(
        parse_period_playlist(PeriodNum::new(0), ArchiveContainer::Ts, text.as_bytes()),
        Err(PlaylistError::DuplicateEndlist)
    ));
}

#[test]
fn a_key_is_rejected_through_the_typed_decoder() {
    let text = ts_playlist(&format!(
        "#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n{}",
        ts_record(0, 0)
    ));
    assert!(matches!(
        parse_period_playlist(PeriodNum::new(0), ArchiveContainer::Ts, text.as_bytes()),
        Err(PlaylistError::Unsupported {
            index: 0,
            tag: UnsupportedTag::Key
        })
    ));
}
