//! The plan contract against an in-memory store modelled on the `fs` adapter
//! (round-2 regression probes 1-4).

use std::collections::BTreeMap;
use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use dvb_si::epg::PresentEventTracker;
use dvr_runtime::read::parse_period_playlist;
use dvr_runtime::writer::{
    ArchiveWriter, InitGeneration, PlanOutcome, StoreFailure, StoreOp, WriterEvent,
};
use dvr_runtime::{ArchiveContainer, PeriodNum};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentCursorItem, SegmentEntry};

use crate::support::{config, periods, present_event, ts_at};

#[derive(Default)]
struct Store {
    data: BTreeMap<u32, Vec<u8>>,
    playlists: BTreeMap<u32, Vec<u8>>,
}

impl Store {
    /// Execute `ops`, modelling the `fs` store: `create_new` for every create
    /// and truncate-to-expected for a length mismatch. `crash_after` executes
    /// that op and then abandons the rest of the plan (a process crash, with
    /// no `complete`).
    fn exec(
        &mut self,
        ops: &[StoreOp],
        fail_at: Option<usize>,
        crash_after: Option<usize>,
    ) -> PlanOutcome {
        for (index, op) in ops.iter().enumerate() {
            if Some(index) == fail_at {
                return PlanOutcome::Failed {
                    at_op: index,
                    failure: StoreFailure::Io(std::io::ErrorKind::Other),
                };
            }
            match op {
                StoreOp::CreateData { period } => {
                    assert!(
                        !self.data.contains_key(&period.value()),
                        "create_new must never replace period {period}"
                    );
                    self.data.insert(period.value(), Vec::new());
                }
                StoreOp::CreatePlaylist { period, text } => {
                    assert!(
                        !self.playlists.contains_key(&period.value()),
                        "create_new must never replace period {period}'s playlist"
                    );
                    self.playlists
                        .insert(period.value(), text.clone().into_bytes());
                }
                StoreOp::AppendData {
                    period,
                    expect_len,
                    bytes,
                } => {
                    let file = self.data.get_mut(&period.value()).expect("data exists");
                    if file.len() as u64 != *expect_len {
                        file.truncate(*expect_len as usize);
                        return PlanOutcome::Failed {
                            at_op: index,
                            failure: StoreFailure::LengthMismatch {
                                actual: file.len() as u64,
                            },
                        };
                    }
                    file.extend_from_slice(bytes);
                }
                StoreOp::AppendPlaylist {
                    period,
                    expect_len,
                    text,
                } => {
                    let file = self
                        .playlists
                        .get_mut(&period.value())
                        .expect("playlist exists");
                    if file.len() as u64 != *expect_len {
                        file.truncate(*expect_len as usize);
                        return PlanOutcome::Failed {
                            at_op: index,
                            failure: StoreFailure::LengthMismatch {
                                actual: file.len() as u64,
                            },
                        };
                    }
                    file.extend_from_slice(text.as_bytes());
                }
                StoreOp::Remove { file } => match file {
                    dvr_runtime::layout::ArchiveFile::Playlist(period) => {
                        self.playlists.remove(&period.value());
                    }
                    dvr_runtime::layout::ArchiveFile::Media(period) => {
                        self.data.remove(&period.value());
                    }
                    _ => {}
                },
                StoreOp::CreateDir | StoreOp::SyncData { .. } => {}
                _ => {}
            }
            if Some(index) == crash_after {
                return PlanOutcome::Done;
            }
        }
        PlanOutcome::Done
    }

    fn execute(
        &mut self,
        writer: &mut ArchiveWriter,
        step: dvr_runtime::writer::Step,
        fail_at: Option<usize>,
    ) -> Vec<StoreOp> {
        match step {
            dvr_runtime::writer::Step::Execute(plan) => {
                let outcome = self.exec(&plan.ops, fail_at, None);
                writer.complete(plan.id, outcome).expect("complete");
                plan.ops
            }
            _ => Vec::new(),
        }
    }

    fn listed(&self, period: u32, container: ArchiveContainer) -> Vec<u32> {
        let text = self.playlists.get(&period).expect("a playlist");
        parse_period_playlist(PeriodNum::new(period), container, text)
            .expect("the playlist parses")
            .segments
            .iter()
            .map(|segment| segment.seq.value())
            .collect()
    }
}

fn entry(seq: u32, len: usize, discontinuous: bool) -> SegmentEntry {
    SegmentEntry::new(
        Bytes::from(vec![seq as u8; len]),
        SegmentSeq::new(seq),
        Duration::from_secs(2),
        Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
        transmux::SegmentMeta { discontinuous },
    )
}

fn item(seq: u32, len: usize, discontinuous: bool) -> SegmentCursorItem {
    SegmentCursorItem::Segment(entry(seq, len, discontinuous))
}

fn long_item(seq: u32) -> SegmentCursorItem {
    SegmentCursorItem::Segment(SegmentEntry::new(
        Bytes::from(vec![seq as u8; 100]),
        SegmentSeq::new(seq),
        Duration::from_secs(5),
        Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
        transmux::SegmentMeta {
            discontinuous: false,
        },
    ))
}

#[test]
fn a_failed_roll_never_publishes_bytes_under_another_sequence() {
    let mut writer =
        ArchiveWriter::new(config(ArchiveContainer::Ts, 2, periods(10))).expect("writer");
    let mut store = Store::default();
    let now = Timestamp::ZERO;
    for seq in 0..2 {
        let step = writer
            .on_item(now, None, &item(seq, 100, false))
            .expect("segment");
        store.execute(&mut writer, step, None);
    }
    // A 5 s segment exceeds the 2 s target and rolls.
    let step = writer.on_item(now, None, &long_item(2)).expect("roll");
    let ops = store.execute(&mut writer, step, Some(0));
    assert!(
        matches!(ops[0], StoreOp::AppendPlaylist { .. }),
        "the roll closes the old period first"
    );
    assert!(
        !store.playlists.contains_key(&1),
        "a failed close must create no new period files"
    );

    let step = writer
        .on_item(now, None, &item(3, 100, false))
        .expect("next");
    let dvr_runtime::writer::Step::Execute(plan) = step else {
        panic!("expected a plan");
    };
    let append = plan
        .ops
        .iter()
        .position(|op| matches!(op, StoreOp::AppendData { .. }))
        .expect("an append");
    let _ = store.exec(&plan.ops, None, Some(append));
    // The process crashed before the playlist record landed.
    if let Some(text) = store.playlists.get(&1) {
        let parsed = parse_period_playlist(PeriodNum::new(1), ArchiveContainer::Ts, text)
            .expect("any published playlist parses");
        let file = &store.data[&1];
        for segment in parsed.segments {
            let start = segment.span.offset as usize;
            let end = (start + segment.span.len.get() as usize).min(file.len());
            assert!(
                file[start..end]
                    .iter()
                    .all(|&b| b == segment.seq.value() as u8),
                "period 1 lists seq {} but its bytes are not that segment's",
                segment.seq
            );
        }
    }
}

#[test]
fn a_close_that_lost_only_its_sync_never_gets_a_record_after_endlist() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Ts, 2, periods(10))
            .with_period_duration(Some(Duration::from_secs(4))),
    )
    .expect("writer");
    let mut store = Store::default();
    let step = writer
        .on_item(Timestamp::ZERO, None, &item(0, 100, false))
        .expect("segment");
    store.execute(&mut writer, step, None);
    let step = writer
        .on_tick(Timestamp::from_nanos(5_000_000_000))
        .expect("tick");
    let ops = store.execute(&mut writer, step, Some(1));
    assert!(matches!(ops[0], StoreOp::AppendPlaylist { .. }));
    let step = writer
        .on_item(
            Timestamp::from_nanos(5_000_000_000),
            None,
            &item(1, 100, false),
        )
        .expect("next");
    store.execute(&mut writer, step, None);

    let text = String::from_utf8(store.playlists[&0].clone()).expect("utf8");
    assert_eq!(text.matches("#EXT-X-ENDLIST").count(), 1, "{text}");
    let after_endlist: Vec<&str> = text
        .lines()
        .skip_while(|line| *line != "#EXT-X-ENDLIST")
        .skip(1)
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .collect();
    assert!(after_endlist.is_empty(), "a record follows ENDLIST: {text}");
    let parsed = parse_period_playlist(PeriodNum::new(0), ArchiveContainer::Ts, text.as_bytes())
        .expect("the closed period reads");
    assert_eq!(parsed.segments.len(), 1);
    assert!(parsed.closed);
}

#[test]
fn a_failed_programme_transition_is_kept_and_retried() {
    let mut writer = ArchiveWriter::new(
        config(ArchiveContainer::Ts, 2, periods(10)).with_programme_service(1537),
    )
    .expect("writer");
    let mut tracker = PresentEventTracker::new(1537);
    let mut store = Store::default();
    let a = present_event(
        &mut tracker,
        1537,
        1,
        "A",
        ts_at(0),
        Duration::from_secs(60),
    );
    writer.on_programme(a).expect("baseline");
    let step = writer
        .on_item(Timestamp::ZERO, Some(ts_at(0)), &item(0, 100, false))
        .expect("segment");
    store.execute(&mut writer, step, None);
    let b = present_event(
        &mut tracker,
        1537,
        2,
        "B",
        ts_at(2),
        Duration::from_secs(60),
    );
    let step = writer.on_programme(b.clone()).expect("transition");
    store.execute(&mut writer, step, Some(0));
    assert_eq!(
        writer.programme().map(|p| p.event.event_id),
        Some(1),
        "a failed close must not adopt the new programme"
    );
    let step = writer.on_programme(b).expect("retry");
    assert!(
        matches!(step, dvr_runtime::writer::Step::Execute(_)),
        "the same transition must be replannable"
    );
    store.execute(&mut writer, step, None);
    let step = writer
        .on_item(Timestamp::ZERO, Some(ts_at(2)), &item(1, 100, false))
        .expect("new period");
    store.execute(&mut writer, step, None);
    let text = String::from_utf8(store.playlists[&1].clone()).expect("utf8");
    assert!(text.contains("X-DVB-EVENT-ID=2"), "{text}");
}

#[test]
fn an_init_change_rolls_at_its_own_first_sequence() {
    let mut writer =
        ArchiveWriter::new(config(ArchiveContainer::Fmp4, 2, periods(10))).expect("writer");
    let mut store = Store::default();
    writer
        .on_inits(&[
            InitGeneration::new(SegmentSeq::new(0), Bytes::from_static(b"INIT1")),
            InitGeneration::new(SegmentSeq::new(5), Bytes::from_static(b"INIT2")),
        ])
        .expect("inits");
    for seq in 0..10 {
        let step = writer
            .on_item(Timestamp::ZERO, None, &item(seq, 100, false))
            .expect("segment");
        let ops = store.execute(&mut writer, step, None);
        assert!(!ops.is_empty(), "segment {seq} planned nothing");
    }
    let events: Vec<WriterEvent> = std::iter::from_fn(|| writer.poll_event()).collect();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, WriterEvent::SkippedBeforeInit { .. })),
        "{events:?}"
    );
    assert_eq!(store.listed(0, ArchiveContainer::Fmp4), vec![0, 1, 2, 3, 4]);
    assert_eq!(store.listed(1, ArchiveContainer::Fmp4), vec![5, 6, 7, 8, 9]);
    assert_eq!(
        events.iter().find_map(|e| match e {
            WriterEvent::PeriodOpened { period, reason } if *period == PeriodNum::new(1) => {
                Some(*reason)
            }
            _ => None,
        }),
        Some(dvr_runtime::RollReason::InitChanged)
    );
    assert!(store.data[&1].starts_with(b"INIT2"));
}
