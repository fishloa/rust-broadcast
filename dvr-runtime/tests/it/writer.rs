//! Writer state-machine tests: every plan is asserted without a disk.

use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use dvb_si::epg::PresentEventTracker;
use dvr_runtime::writer::{
    InitGeneration, PlanOutcome, RollReason, Step, StoreFailure, StoreOp, WriterEvent,
};
use dvr_runtime::{ArchiveContainer, ArchiveError, ArchiveWriter, PeriodNum};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{ArchiveOverrun, SegmentCursorItem};

use crate::support::{
    append_playlist_index, bytes_limit, config, events, evicted, opened_events, periods, plan,
    playlist_text, present_event, segment_at, step_item, step_item_fail, ts_at,
};

const INIT: [u8; 4] = *b"INIT";

fn fmp4_writer(retention: dvr_runtime::RetentionLimits) -> ArchiveWriter {
    ArchiveWriter::new(config(ArchiveContainer::Fmp4, 3, retention)).expect("writer")
}

fn with_init(writer: &mut ArchiveWriter, first_seq: u32) {
    writer
        .on_inits(&[InitGeneration::new(
            SegmentSeq::new(first_seq),
            Bytes::from(INIT.to_vec()),
        )])
        .expect("inits");
}

fn item(seq: u32, len: usize, at_ns: u64, discontinuous: bool) -> SegmentCursorItem {
    SegmentCursorItem::Segment(segment_at(seq, len, at_ns, discontinuous))
}

#[test]
fn first_record_creates_the_playlist_and_later_records_append_one() {
    let mut writer = fmp4_writer(periods(5));
    with_init(&mut writer, 0);
    let (_, first) = step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    assert!(matches!(first[0], StoreOp::CreateDir));
    assert!(matches!(first[1], StoreOp::CreateData { period } if period == PeriodNum::new(0)));
    let create = first
        .iter()
        .find_map(|op| match op {
            StoreOp::CreatePlaylist { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("create playlist");
    assert!(create.starts_with("#EXTM3U\n"));
    assert_eq!(create.matches("#EXT-X-MEDIA-SEQUENCE:0").count(), 1);
    assert_eq!(create.matches(".m4s\n").count(), 1);

    let (_, second) = step_item(
        &mut writer,
        Timestamp::ZERO,
        None,
        item(1, 100, 2_000_000_000, false),
    );
    let (append, expect_len) = second
        .iter()
        .find_map(|op| match op {
            StoreOp::AppendPlaylist {
                text, expect_len, ..
            } => Some((text.clone(), *expect_len)),
            _ => None,
        })
        .expect("append playlist");
    assert_eq!(
        expect_len,
        create.len() as u64,
        "the append continues the created file"
    );
    assert!(
        !append.starts_with("#EXTM3U"),
        "a later record is one record, no header"
    );
    assert_eq!(append.matches(".m4s\n").count(), 1);

    let mut text = create;
    text.push_str(&append);
    let parsed = dvr_runtime::read::parse_period_playlist(
        PeriodNum::new(0),
        ArchiveContainer::Fmp4,
        text.as_bytes(),
    )
    .expect("parses back");
    assert_eq!(parsed.segments.len(), 2);
    assert_eq!(parsed.segments[1].seq, SegmentSeq::new(1));
}

#[test]
fn a_sequence_beyond_u32_is_refused_without_a_plan() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Fmp4, 3, periods(5)).with_seq_offset(u64::from(u32::MAX)),
    )
    .expect("writer");
    with_init(&mut writer, 0);
    let error = writer
        .on_item(Timestamp::ZERO, None, &item(1, 100, 0, false))
        .expect_err("must overflow");
    assert!(matches!(
        error,
        ArchiveError::SeqOverflow {
            trunk_seq,
            offset
        } if trunk_seq == SegmentSeq::new(1) && offset == u64::from(u32::MAX)
    ));
    assert_eq!(writer.counters().segments, 0);
}

#[test]
fn an_empty_segment_is_refused() {
    let mut writer = fmp4_writer(periods(5));
    with_init(&mut writer, 0);
    let error = writer
        .on_item(Timestamp::ZERO, None, &item(0, 0, 0, false))
        .expect_err("must refuse");
    assert!(matches!(
        error,
        ArchiveError::EmptySegment { seq } if seq == SegmentSeq::new(0)
    ));
}

#[test]
fn a_failed_segment_plan_does_not_advance_the_sequence() {
    let mut writer = fmp4_writer(periods(5));
    with_init(&mut writer, 0);
    let (_, first) = step_item_fail(
        &mut writer,
        Timestamp::ZERO,
        None,
        item(0, 100, 0, false),
        0,
        StoreFailure::Io(std::io::ErrorKind::Other),
    );
    assert!(
        first
            .iter()
            .any(|op| matches!(op, StoreOp::CreatePlaylist { .. }))
    );
    assert_eq!(writer.counters().segments, 0);
    let (_, retry) = step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    assert!(
        retry
            .iter()
            .any(|op| matches!(op, StoreOp::CreatePlaylist { .. }))
    );
}

#[test]
fn a_failed_roll_commits_nothing_and_keeps_the_period_open() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Fmp4, 3, periods(5))
            .with_period_duration(Some(Duration::from_secs(1))),
    )
    .expect("writer");
    with_init(&mut writer, 0);
    step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    let close = plan(
        writer
            .on_tick(Timestamp::from_nanos(2_000_000_000))
            .expect("tick"),
    );
    assert!(matches!(close.ops[0], StoreOp::AppendPlaylist { .. }));
    writer
        .complete(
            close.id,
            PlanOutcome::Failed {
                at_op: 0,
                failure: StoreFailure::Io(std::io::ErrorKind::Other),
            },
        )
        .expect("complete");
    let (_, next) = step_item(
        &mut writer,
        Timestamp::from_nanos(2_000_000_000),
        None,
        item(1, 100, 2_000_000_000, false),
    );
    assert!(matches!(next[0], StoreOp::AppendData { period, .. } if period == PeriodNum::new(0)));
    assert!(
        !next
            .iter()
            .any(|op| matches!(op, StoreOp::CreateData { .. }))
    );
}

#[test]
fn a_playlist_length_mismatch_resyncs_the_next_append() {
    let mut writer = fmp4_writer(periods(5));
    with_init(&mut writer, 0);
    let (_, create) = step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    let created_len = playlist_text(&create).len() as u64;
    let step = writer
        .on_item(Timestamp::ZERO, None, &item(1, 100, 2_000_000_000, false))
        .expect("second");
    let plan = plan(step);
    let at_op = append_playlist_index(&plan.ops);
    let truncated = created_len - 10;
    writer
        .complete(
            plan.id,
            PlanOutcome::Failed {
                at_op,
                failure: StoreFailure::LengthMismatch { actual: truncated },
            },
        )
        .expect("complete");
    let (_, retry) = step_item(
        &mut writer,
        Timestamp::ZERO,
        None,
        item(1, 100, 2_000_000_000, false),
    );
    let expect_len = retry
        .iter()
        .find_map(|op| match op {
            StoreOp::AppendPlaylist { expect_len, .. } => Some(*expect_len),
            _ => None,
        })
        .expect("append");
    assert_eq!(expect_len, truncated);
}

#[test]
fn eviction_removes_the_playlist_before_the_media_file() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Fmp4, 3, periods(1))
            .with_period_duration(Some(Duration::from_secs(1))),
    )
    .expect("writer");
    with_init(&mut writer, 0);
    step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    let close = plan(
        writer
            .on_tick(Timestamp::from_nanos(2_000_000_000))
            .expect("tick"),
    );
    writer
        .complete(close.id, PlanOutcome::Done)
        .expect("complete");
    step_item(
        &mut writer,
        Timestamp::from_nanos(2_000_000_000),
        None,
        item(1, 100, 2_000_000_000, false),
    );
    let close = plan(
        writer
            .on_tick(Timestamp::from_nanos(4_000_000_000))
            .expect("tick"),
    );
    writer
        .complete(close.id, PlanOutcome::Done)
        .expect("complete");
    let (_, next) = step_item(
        &mut writer,
        Timestamp::from_nanos(4_000_000_000),
        None,
        item(2, 100, 4_000_000_000, false),
    );
    let removes: Vec<StoreOp> = next
        .into_iter()
        .filter(|op| matches!(op, StoreOp::Remove { .. }))
        .collect();
    assert_eq!(removes.len(), 2);
    assert!(matches!(
        removes[0],
        StoreOp::Remove {
            file: dvr_runtime::layout::ArchiveFile::Playlist(period)
        } if period == PeriodNum::new(0)
    ));
    assert!(matches!(
        removes[1],
        StoreOp::Remove {
            file: dvr_runtime::layout::ArchiveFile::Media(period)
        } if period == PeriodNum::new(0)
    ));
    assert!(events(&mut writer).contains(&WriterEvent::Evicted {
        period: PeriodNum::new(0)
    }));
}

#[test]
fn byte_budget_keeps_the_configured_number_of_finished_periods() {
    let mut writer = ArchiveWriter::new(config(
        ArchiveContainer::Fmp4,
        3,
        dvr_runtime::RetentionLimits::new()
            .with_periods(std::num::NonZeroUsize::new(3).expect("nonzero"))
            .with_bytes(std::num::NonZeroU64::new(3000).expect("nonzero")),
    ))
    .expect("writer");
    with_init(&mut writer, 0);
    for seq in 0..30u32 {
        let (_, ops) = step_item(
            &mut writer,
            Timestamp::ZERO,
            None,
            item(seq, 100, u64::from(seq) * 1_000_000_000, false),
        );
        assert!(!ops.is_empty());
    }
    let log = events(&mut writer);
    let opened = opened_events(&log).len();
    let evicted = evicted(&log).len();
    assert_eq!(
        opened - evicted - 1,
        3,
        "three finished periods are kept (opened {opened}, evicted {evicted}, log {log:?})"
    );
}

#[test]
fn byte_retention_counts_the_open_period() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Fmp4, 3, bytes_limit(4096))
            .with_period_duration(Some(Duration::from_secs(86400))),
    )
    .expect("writer");
    with_init(&mut writer, 0);
    step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    step_item(
        &mut writer,
        Timestamp::ZERO,
        None,
        item(1, 2000, 2_000_000_000, false),
    );
    for seq in 2..5u32 {
        step_item(
            &mut writer,
            Timestamp::ZERO,
            None,
            item(seq, 2000, u64::from(seq) * 2_000_000_000, false),
        );
    }
    let log = events(&mut writer);
    assert!(
        evicted(&log).contains(&PeriodNum::new(0)),
        "the open period's own bytes must force p0 out: {log:?}"
    );
}

#[test]
fn a_byte_capped_route_with_no_time_roll_never_exceeds_the_budget() {
    let mut writer = ArchiveWriter::new(
        config(
            ArchiveContainer::Fmp4,
            3,
            dvr_runtime::RetentionLimits::new()
                .with_periods(std::num::NonZeroUsize::new(2).expect("nonzero"))
                .with_bytes(std::num::NonZeroU64::new(4096).expect("nonzero")),
        )
        .with_programme_service(1),
    )
    .expect("writer");
    with_init(&mut writer, 0);
    let mut observed = std::collections::BTreeMap::<PeriodNum, u64>::new();
    for seq in 0..12u32 {
        let (_, ops) = step_item(
            &mut writer,
            Timestamp::ZERO,
            Some(ts_at(0)),
            item(seq, 1000, u64::from(seq) * 1_000_000_000, false),
        );
        for op in &ops {
            if let StoreOp::AppendData { period, bytes, .. } = op {
                *observed.entry(*period).or_default() += bytes.len() as u64;
            }
        }
    }
    assert!(observed.len() > 1, "a byte-capped route must roll by bytes");
    for (period, bytes) in observed {
        assert!(
            bytes <= 1365 + 1000,
            "period {period} holds {bytes} bytes, over 1365 + one segment"
        );
    }
}

#[test]
fn one_oversized_segment_does_not_roll_an_empty_period() {
    let mut writer = fmp4_writer(
        dvr_runtime::RetentionLimits::new()
            .with_periods(std::num::NonZeroUsize::new(2).expect("nonzero"))
            .with_bytes(std::num::NonZeroU64::new(64).expect("nonzero")),
    );
    with_init(&mut writer, 0);
    let (_, ops) = step_item(&mut writer, Timestamp::ZERO, None, item(0, 200, 0, false));
    assert!(ops.iter().any(
        |op| matches!(op, StoreOp::CreatePlaylist { period, .. } if *period == PeriodNum::new(0))
    ));
    assert!(
        !ops.iter()
            .any(|op| matches!(op, StoreOp::CreateData { period } if *period != PeriodNum::new(0)))
    );
}

#[test]
fn hard_cap_rolls_on_clock_even_with_programme_tracking_never_fed() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Ts, 2, periods(8))
            .with_period_duration(Some(Duration::from_secs(2)))
            .with_programme_service(0x0601),
    )
    .expect("writer");
    step_item(
        &mut writer,
        Timestamp::ZERO,
        Some(ts_at(0)),
        item(0, 100, 0, false),
    );
    let close = plan(
        writer
            .on_tick(Timestamp::from_nanos(3_000_000_000))
            .expect("tick"),
    );
    writer
        .complete(close.id, PlanOutcome::Done)
        .expect("complete");
    step_item(
        &mut writer,
        Timestamp::from_nanos(3_000_000_000),
        Some(ts_at(4)),
        item(1, 100, 2_000_000_000, false),
    );
    let log = events(&mut writer);
    let opened = opened_events(&log);
    assert_eq!(opened[0], (PeriodNum::new(0), RollReason::First));
    assert_eq!(opened[1], (PeriodNum::new(1), RollReason::Duration));
}

#[test]
fn needs_poll_tracks_new_segments_and_new_inits() {
    let mut writer = fmp4_writer(periods(5));
    assert!(!writer.needs_poll(None, None));
    assert!(writer.needs_poll(Some(SegmentSeq::new(0)), None));
    assert!(writer.needs_poll(None, Some(&INIT)));
    writer
        .on_inits(&[InitGeneration::new(
            SegmentSeq::new(0),
            Bytes::from(INIT.to_vec()),
        )])
        .expect("inits");
    assert!(!writer.needs_poll(None, Some(&INIT)));
    step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    assert!(!writer.needs_poll(Some(SegmentSeq::new(0)), Some(&INIT)));
    assert!(writer.needs_poll(Some(SegmentSeq::new(1)), Some(&INIT)));
}

#[test]
fn gap_and_lagged_accounting() {
    let mut writer = fmp4_writer(periods(5));
    writer
        .on_item(
            Timestamp::ZERO,
            None,
            &SegmentCursorItem::Gap { skipped: 3 },
        )
        .expect("gap");
    writer
        .on_item(
            Timestamp::ZERO,
            None,
            &SegmentCursorItem::Lagged { skipped: 2 },
        )
        .expect("lagged");
    assert_eq!(writer.counters().gaps, 5);
    let log = events(&mut writer);
    assert!(log.contains(&WriterEvent::Gap { skipped: 3 }));
    assert!(log.contains(&WriterEvent::Gap { skipped: 2 }));

    with_init(&mut writer, 0);
    step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    assert_eq!(opened_events(&events(&mut writer))[0].1, RollReason::First);
    step_item(
        &mut writer,
        Timestamp::ZERO,
        None,
        item(5, 100, 5_000_000_000, false),
    );
    assert_eq!(
        opened_events(&events(&mut writer))[0],
        (PeriodNum::new(1), RollReason::SequenceGap)
    );
}

#[test]
fn stall_termination_rearms_with_the_exact_skip() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Ts, 2, periods(5)).with_overrun(ArchiveOverrun::StallIngest),
    )
    .expect("writer");
    assert_eq!(writer.overrun(), ArchiveOverrun::StallIngest);
    with_init(&mut writer, 0);
    step_item(&mut writer, Timestamp::ZERO, None, item(1, 100, 0, false));
    writer.set_latest_closed(Some(SegmentSeq::new(4)));
    let step = writer
        .on_item(Timestamp::ZERO, None, &SegmentCursorItem::Terminated)
        .expect("terminated");
    assert!(matches!(step, Step::Rearm { skipped: 3 }));
    assert_eq!(writer.counters().gaps, 3);
    assert!(events(&mut writer).contains(&WriterEvent::Rearmed { skipped: 3 }));

    let mut terminate =
        ArchiveWriter::new(config(ArchiveContainer::Ts, 2, periods(5))).expect("writer");
    let step = terminate
        .on_item(Timestamp::ZERO, None, &SegmentCursorItem::Terminated)
        .expect("terminated");
    assert!(matches!(step, Step::Stopped));
    assert!(matches!(
        terminate.on_item(Timestamp::ZERO, None, &item(0, 10, 0, false)),
        Ok(Step::Idle)
    ));
}

#[test]
fn a_repeated_present_event_rolls_nothing_and_adds_no_second_daterange() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Fmp4, 3, periods(5)).with_programme_service(1537),
    )
    .expect("writer");
    with_init(&mut writer, 0);
    let mut tracker = PresentEventTracker::new(1537);
    let baseline = present_event(
        &mut tracker,
        1537,
        30807,
        "Title",
        ts_at(0),
        Duration::from_secs(60),
    );
    assert!(matches!(
        writer.on_programme(baseline).expect("baseline"),
        Step::Idle
    ));
    let (_, first) = step_item(
        &mut writer,
        Timestamp::ZERO,
        Some(ts_at(1)),
        item(0, 100, 0, false),
    );
    assert_eq!(
        playlist_text(&first).matches("#EXT-X-DATERANGE:").count(),
        1
    );

    let mut same = PresentEventTracker::new(1537);
    let again = present_event(
        &mut same,
        1537,
        30807,
        "Title",
        ts_at(0),
        Duration::from_secs(60),
    );
    assert!(matches!(
        writer.on_programme(again).expect("again"),
        Step::Idle
    ));
    let (_, second) = step_item(
        &mut writer,
        Timestamp::ZERO,
        Some(ts_at(3)),
        item(1, 100, 2_000_000_000, false),
    );
    assert_eq!(
        playlist_text(&second).matches("#EXT-X-DATERANGE:").count(),
        0
    );
    assert!(
        !events(&mut writer)
            .iter()
            .any(|event| matches!(event, WriterEvent::PeriodClosed { .. }))
    );
}

#[test]
fn a_programme_transition_closes_the_open_period() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Fmp4, 3, periods(5)).with_programme_service(1537),
    )
    .expect("writer");
    with_init(&mut writer, 0);
    let mut tracker = PresentEventTracker::new(1537);
    let baseline = present_event(
        &mut tracker,
        1537,
        30807,
        "One",
        ts_at(0),
        Duration::from_secs(60),
    );
    writer.on_programme(baseline).expect("baseline");
    step_item(
        &mut writer,
        Timestamp::ZERO,
        Some(ts_at(1)),
        item(0, 100, 0, false),
    );
    let transition = present_event(
        &mut tracker,
        1537,
        30808,
        "Two",
        ts_at(60),
        Duration::from_secs(60),
    );
    let plan = plan(writer.on_programme(transition).expect("transition"));
    assert!(matches!(plan.ops[0], StoreOp::AppendPlaylist { .. }));
    writer
        .complete(plan.id, PlanOutcome::Done)
        .expect("complete");
    let (_, next) = step_item(
        &mut writer,
        Timestamp::ZERO,
        Some(ts_at(61)),
        item(1, 100, 2_000_000_000, false),
    );
    let text = playlist_text(&next);
    assert!(text.contains("X-DVB-EVENT-ID=30808"), "{text}");
    assert_eq!(
        opened_events(&events(&mut writer)).last().copied(),
        Some((PeriodNum::new(1), RollReason::Programme))
    );
}

#[test]
fn seed_moves_the_next_period_past_every_existing_one() {
    let mut writer = fmp4_writer(periods(5));
    let mut existing = Vec::new();
    for n in [0u32, 1, 2] {
        existing.push(dvr_runtime::layout::SeedFile::new(
            dvr_runtime::layout::ArchiveFile::Media(PeriodNum::new(n)),
            1000,
        ));
        existing.push(dvr_runtime::layout::SeedFile::new(
            dvr_runtime::layout::ArchiveFile::Playlist(PeriodNum::new(n)),
            200,
        ));
    }
    writer.seed(&existing).expect("seed");
    with_init(&mut writer, 0);
    let (_, ops) = step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    assert!(
        ops.iter()
            .any(|op| matches!(op, StoreOp::CreateData { period } if *period == PeriodNum::new(3)))
    );
    let create = playlist_text(&ops);
    assert_eq!(create.matches("#EXT-X-DISCONTINUITY").count(), 1);
}

#[test]
fn seeding_the_last_period_number_exhausts_the_range() {
    let mut writer = fmp4_writer(periods(5));
    let media_only = [dvr_runtime::layout::SeedFile::new(
        dvr_runtime::layout::ArchiveFile::Media(PeriodNum::new(u32::MAX)),
        10,
    )];
    writer
        .seed(&media_only)
        .expect("a stray media file with no playlist is not a period");
    let playlist = [dvr_runtime::layout::SeedFile::new(
        dvr_runtime::layout::ArchiveFile::Playlist(PeriodNum::new(u32::MAX)),
        10,
    )];
    let mut writer = fmp4_writer(periods(5));
    assert!(matches!(
        writer.seed(&playlist),
        Err(ArchiveError::PeriodNumbersExhausted)
    ));
}

#[test]
fn an_idle_writer_never_asks_to_be_polled() {
    let writer = fmp4_writer(periods(5));
    assert!(
        !writer.needs_poll(None, None),
        "nothing new must be false, never a busy loop"
    );
}

#[test]
fn a_later_segment_does_not_restart_the_period_clock() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Ts, 2, periods(5))
            .with_period_duration(Some(Duration::from_secs(4))),
    )
    .expect("writer");
    with_init(&mut writer, 0);
    step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    step_item(
        &mut writer,
        Timestamp::from_nanos(3_000_000_000),
        None,
        item(1, 100, 2_000_000_000, false),
    );
    let step = writer
        .on_tick(Timestamp::from_nanos(4_000_000_000))
        .expect("tick");
    assert!(
        matches!(step, Step::Execute(_)),
        "the period's clock runs from when it opened, not from its last segment"
    );
}

#[test]
fn a_failed_removal_keeps_the_period_and_emits_the_failure() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Fmp4, 3, periods(1))
            .with_period_duration(Some(Duration::from_secs(1))),
    )
    .expect("writer");
    with_init(&mut writer, 0);
    step_item(&mut writer, Timestamp::ZERO, None, item(0, 100, 0, false));
    let close = plan(
        writer
            .on_tick(Timestamp::from_nanos(2_000_000_000))
            .expect("tick"),
    );
    writer
        .complete(close.id, PlanOutcome::Done)
        .expect("complete");
    step_item(
        &mut writer,
        Timestamp::from_nanos(2_000_000_000),
        None,
        item(1, 100, 2_000_000_000, false),
    );
    let close = plan(
        writer
            .on_tick(Timestamp::from_nanos(4_000_000_000))
            .expect("tick"),
    );
    writer
        .complete(close.id, PlanOutcome::Done)
        .expect("complete");
    let step = writer
        .on_item(
            Timestamp::from_nanos(4_000_000_000),
            None,
            &item(2, 100, 4_000_000_000, false),
        )
        .expect("third period");
    let plan = plan(step);
    let first_remove = plan
        .ops
        .iter()
        .position(|op| matches!(op, StoreOp::Remove { .. }))
        .expect("a removal");
    writer
        .complete(
            plan.id,
            PlanOutcome::Failed {
                at_op: first_remove,
                failure: StoreFailure::Io(std::io::ErrorKind::Other),
            },
        )
        .expect("complete");
    let log = events(&mut writer);
    assert!(
        log.contains(&WriterEvent::EvictionFailed {
            period: PeriodNum::new(0)
        }),
        "{log:?}"
    );
    assert!(!evicted(&log).contains(&PeriodNum::new(0)));
}

#[test]
fn an_outstanding_plan_blocks_every_input() {
    let mut writer = fmp4_writer(periods(5));
    with_init(&mut writer, 0);
    let pending = plan(
        writer
            .on_item(Timestamp::ZERO, None, &item(0, 100, 0, false))
            .expect("first"),
    );
    assert!(matches!(
        writer.on_item(Timestamp::ZERO, None, &item(1, 100, 2_000_000_000, false)),
        Err(ArchiveError::PlanPending)
    ));
    assert!(matches!(
        writer.on_tick(Timestamp::ZERO),
        Err(ArchiveError::PlanPending)
    ));
    assert!(matches!(writer.seed(&[]), Err(ArchiveError::PlanPending)));
    writer
        .complete(
            pending.id,
            PlanOutcome::Failed {
                at_op: 0,
                failure: StoreFailure::Io(std::io::ErrorKind::Other),
            },
        )
        .expect("complete");
    assert!(matches!(
        writer.complete(pending.id, PlanOutcome::Done),
        Err(ArchiveError::UnknownPlan { .. })
    ));
}

#[test]
fn roll_reasons_have_stable_labels() {
    assert_eq!(RollReason::First.name(), "first");
    assert_eq!(RollReason::InitChanged.name(), "init_changed");
    assert_eq!(RollReason::PlaylistFull.to_string(), "playlist_full");
    assert_eq!(ArchiveContainer::Fmp4.name(), "FMP4");
    assert_eq!(ArchiveContainer::Ts.to_string(), "TS");
}
