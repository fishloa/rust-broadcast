//! The committed period-playlist golden (DESIGN §3.7).
//!
//! The golden is produced once by the writer from the fixed input below and
//! reviewed against the design's example; the test only ever compares.

use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::MediaPlaylist;
use dvb_si::epg::PresentEventTracker;
use dvr_runtime::read::parse_period_playlist;
use dvr_runtime::writer::{InitGeneration, Plan, PlanOutcome, Step};
use dvr_runtime::{ArchiveConfig, ArchiveContainer, ArchiveWriter, PeriodNum, RetentionLimits};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::SegmentCursorItem;

use crate::support::{present_event, segment_at, ts_at};

const SEQ_OFFSET: u64 = 116;

fn input() -> (ArchiveWriter, PresentEventTracker) {
    let retention =
        RetentionLimits::new().with_periods(std::num::NonZeroUsize::new(5).expect("nonzero"));
    let config = ArchiveConfig::new(ArchiveContainer::Fmp4, Duration::from_secs(3), retention)
        .with_period_duration(Some(Duration::from_secs(1)))
        .with_seq_offset(SEQ_OFFSET)
        .with_programme_service(1537);
    let writer = ArchiveWriter::new(config).expect("valid config");
    let tracker = PresentEventTracker::new(1537);
    (writer, tracker)
}

fn commit(writer: &mut ArchiveWriter, step: Step) -> String {
    let Step::Execute(plan) = step else {
        panic!("expected a plan, got {step:?}");
    };
    let text = playlist_text(&plan);
    writer
        .complete(plan.id, PlanOutcome::Done)
        .expect("complete");
    text
}

fn playlist_text(plan: &Plan) -> String {
    let mut text = String::new();
    for op in &plan.ops {
        match op {
            dvr_runtime::writer::StoreOp::CreatePlaylist { text: body, .. }
            | dvr_runtime::writer::StoreOp::AppendPlaylist { text: body, .. } => {
                text.push_str(body)
            }
            _ => {}
        }
    }
    text
}

/// Drive the writer through periods 0..=3 so the golden is period 4 and the
/// first record is not the start of a seeded run.
fn roll_to_period_four(writer: &mut ArchiveWriter) {
    writer
        .on_inits(&[InitGeneration::new(
            SegmentSeq::new(0),
            bytes::Bytes::from(vec![0x49u8; 812]),
        )])
        .expect("inits");
    let mut now = Timestamp::ZERO;
    for seq in 0..4u32 {
        let step = writer
            .on_item(
                now,
                Some(ts_at(u64::from(seq) * 2)),
                &SegmentCursorItem::Segment(segment_at(
                    seq,
                    1000,
                    u64::from(seq) * 2_000_000_000,
                    false,
                )),
            )
            .expect("pre-roll segment");
        let _ = commit(writer, step);
        now = now.saturating_add(Duration::from_secs(2));
        let step = writer.on_tick(now).expect("pre-roll close");
        let _ = commit(writer, step);
    }
}

fn golden_lines(writer: &mut ArchiveWriter, tracker: &mut PresentEventTracker) -> String {
    roll_to_period_four(writer);
    let change = present_event(
        tracker,
        1537,
        30807,
        "Die \"Drei\" Frage",
        ts_at(0),
        Duration::from_secs(3600),
    );
    assert!(matches!(
        writer.on_programme(change).expect("programme"),
        Step::Idle
    ));
    let now = Timestamp::from_nanos(240_000_000_000);
    let step = writer
        .on_item(
            now,
            Some(ts_at(2)),
            &SegmentCursorItem::Segment(segment_at(4, 48_211, 240_000_000_000, false)),
        )
        .expect("first segment");
    let mut text = commit(writer, step);
    let step = writer
        .on_item(
            now,
            Some(ts_at(4)),
            &SegmentCursorItem::Segment(segment_at(5, 51_002, 242_000_000_000, true)),
        )
        .expect("second segment");
    text.push_str(&commit(writer, step));
    let step = writer
        .on_tick(Timestamp::from_nanos(242_000_000_000))
        .expect("tick");
    text.push_str(&commit(writer, step));
    text
}

#[test]
fn period_playlist_golden() {
    let golden = test_fixtures::read_to_string("golden/dvr-runtime/p4.m3u8");
    let (mut writer, mut tracker) = input();
    let text = golden_lines(&mut writer, &mut tracker);
    assert_eq!(text, golden, "the writer's lines must be the golden");

    let parsed =
        parse_period_playlist(PeriodNum::new(4), ArchiveContainer::Fmp4, golden.as_bytes())
            .expect("the golden parses");
    assert_eq!(parsed.segments.len(), 2);
    assert_eq!(parsed.segments[0].seq, SegmentSeq::new(120));
    assert_eq!(parsed.segments[1].seq, SegmentSeq::new(121));
    assert_eq!(
        parsed.segments[0].start,
        Timestamp::from_nanos(240_000_000_000)
    );
    assert_eq!(
        parsed.segments[1].start,
        Timestamp::from_nanos(242_000_000_000)
    );
    assert_eq!(parsed.segments[0].duration, Duration::from_secs(2));
    assert_eq!(parsed.segments[0].span.offset, 812);
    assert_eq!(parsed.segments[0].span.len.get(), 48_211);
    assert_eq!(parsed.segments[1].span.offset, 49_023);
    assert_eq!(parsed.segments[1].span.len.get(), 51_002);
    assert!(!parsed.segments[0].discontinuous);
    assert!(parsed.segments[1].discontinuous);
    assert_eq!(parsed.segments[0].init.expect("map").offset, 0);
    assert_eq!(parsed.segments[0].init.expect("map").len.get(), 812);
    assert!(parsed.closed);
    assert_eq!(parsed.torn_bytes, 0);
    assert_eq!(parsed.torn_records, 0);
    assert_eq!(parsed.programmes.len(), 1);
    assert_eq!(parsed.programmes[0].service_id, 1537);
    assert_eq!(parsed.programmes[0].event_id, 30807);
    assert_eq!(
        parsed.programmes[0].title.as_deref(),
        Some("Die \"Drei\" Frage")
    );
    assert_eq!(
        parsed.programmes[0].planned,
        Some(Duration::from_secs(3600))
    );
    assert_eq!(parsed.programmes[0].first_seq, SegmentSeq::new(120));

    let options = broadcast_hls::ParseOptions::default().with_segment_scoped_tag(
        broadcast_hls::CustomTagName::new(dvr_runtime::format::TIMING_TAG_NAME).expect("tag name"),
    );
    let round = MediaPlaylist::parse_with_options(
        &golden,
        &broadcast_hls::limits::Limits::default(),
        &options,
    )
    .expect("broadcast-hls parses the golden")
    .to_m3u8()
    .expect("renders");
    assert_eq!(round, golden);
}
