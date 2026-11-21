//! Crash, partial-failure and restart regressions, driven through an
//! in-memory store that follows the `fs` adapter's `create_new` and
//! `expect_len` contract.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use dvr_runtime::layout::{ArchiveFile, SeedFile};
use dvr_runtime::read::parse_period_playlist;
use dvr_runtime::writer::{
    InitGeneration, PlanOutcome, Step, StoreFailure, StoreOp, StoreOpKind, WriterEvent,
};
use dvr_runtime::{ArchiveContainer, ArchiveError, ArchiveWriter, PeriodNum};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentCursorItem, SegmentEntry};

use crate::support::{bytes_limit, config, events, periods, plan};

#[derive(Default, Clone)]
pub struct MemStore {
    pub data: BTreeMap<u32, Vec<u8>>,
    pub playlists: BTreeMap<u32, Vec<u8>>,
}

impl MemStore {
    fn op(&mut self, op: &StoreOp) -> Result<(), StoreFailure> {
        match op {
            StoreOp::CreateData { period } => {
                if self.data.contains_key(&period.value()) {
                    return Err(StoreFailure::Io(ErrorKind::AlreadyExists));
                }
                self.data.insert(period.value(), Vec::new());
            }
            StoreOp::AppendData {
                period,
                expect_len,
                bytes,
            } => {
                let file = self
                    .data
                    .get_mut(&period.value())
                    .ok_or(StoreFailure::Io(ErrorKind::NotFound))?;
                if file.len() as u64 != *expect_len {
                    return Err(StoreFailure::LengthMismatch {
                        actual: file.len() as u64,
                    });
                }
                file.extend_from_slice(bytes);
            }
            StoreOp::CreatePlaylist { period, text } => {
                if self.playlists.contains_key(&period.value()) {
                    return Err(StoreFailure::Io(ErrorKind::AlreadyExists));
                }
                self.playlists
                    .insert(period.value(), text.clone().into_bytes());
            }
            StoreOp::AppendPlaylist {
                period,
                expect_len,
                text,
            } => {
                let file = self
                    .playlists
                    .get_mut(&period.value())
                    .ok_or(StoreFailure::Io(ErrorKind::NotFound))?;
                if file.len() as u64 != *expect_len {
                    return Err(StoreFailure::LengthMismatch {
                        actual: file.len() as u64,
                    });
                }
                file.extend_from_slice(text.as_bytes());
            }
            StoreOp::Remove { file } => match file {
                ArchiveFile::Playlist(period) => {
                    self.playlists.remove(&period.value());
                }
                ArchiveFile::Media(period) => {
                    self.data.remove(&period.value());
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    pub fn execute(&mut self, ops: &[StoreOp]) -> PlanOutcome {
        for (at_op, op) in ops.iter().enumerate() {
            if let Err(failure) = self.op(op) {
                return PlanOutcome::Failed { at_op, failure };
            }
        }
        PlanOutcome::Done
    }

    pub fn crash_after(&mut self, ops: &[StoreOp], last: usize) {
        for op in ops.iter().take(last + 1) {
            if self.op(op).is_err() {
                return;
            }
        }
    }

    pub fn seed_files(&self) -> Vec<SeedFile> {
        let mut files = Vec::new();
        for (period, bytes) in &self.data {
            files.push(SeedFile::new(
                ArchiveFile::Media(PeriodNum::new(*period)),
                bytes.len() as u64,
            ));
        }
        for (period, bytes) in &self.playlists {
            files.push(SeedFile::new(
                ArchiveFile::Playlist(PeriodNum::new(*period)),
                bytes.len() as u64,
            ));
        }
        files
    }

    pub fn listed_seqs(&self, container: ArchiveContainer) -> Vec<(u32, u32)> {
        let mut listed = Vec::new();
        for (period, text) in &self.playlists {
            let parsed = parse_period_playlist(PeriodNum::new(*period), container, text)
                .expect("every playlist the writer leaves is readable");
            let media = self.data.get(period).map_or(&[][..], Vec::as_slice);
            for segment in &parsed.segments {
                let start = usize::try_from(segment.span.offset).expect("offset");
                let end = start + usize::try_from(segment.span.len.get()).expect("len");
                assert!(
                    end <= media.len(),
                    "p{period} seq {} beyond file",
                    segment.seq
                );
                let fill = u8::try_from(segment.seq.value()).expect("small seq");
                assert!(
                    media[start..end].iter().all(|b| *b == fill),
                    "p{period} seq {} points at other bytes",
                    segment.seq
                );
                if let Some(init) = segment.init {
                    let init_end = usize::try_from(init.offset + init.len.get()).expect("init");
                    assert!(init_end <= media.len(), "p{period} init beyond file");
                }
                listed.push((*period, segment.seq.value()));
            }
        }
        listed
    }
}

fn seg(seq: u32, secs: u64) -> SegmentCursorItem {
    SegmentCursorItem::Segment(SegmentEntry::new(
        Bytes::from(vec![
            u8::try_from(seq).expect("small seq");
            100 + seq as usize
        ]),
        SegmentSeq::new(seq),
        Duration::from_secs(secs),
        Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
        transmux::SegmentMeta {
            discontinuous: false,
        },
    ))
}

fn create_data_target(ops: &[StoreOp]) -> Option<u32> {
    ops.iter().find_map(|op| match op {
        StoreOp::CreateData { period } => Some(period.value()),
        _ => None,
    })
}

fn removed(ops: &[StoreOp]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match op {
            StoreOp::Remove { file } => Some(match file {
                ArchiveFile::Playlist(p) => format!("p{p}.m3u8"),
                ArchiveFile::Media(p) => format!("p{p}.media"),
                _ => String::from("other"),
            }),
            _ => None,
        })
        .collect()
}

fn run(writer: &mut ArchiveWriter, store: &mut MemStore, item: &SegmentCursorItem) -> Vec<StoreOp> {
    let plan = plan(
        writer
            .on_item(Timestamp::ZERO, None, item)
            .expect("on_item"),
    );
    let outcome = store.execute(&plan.ops);
    writer.complete(plan.id, outcome).expect("complete");
    plan.ops
}

#[test]
fn a_crash_between_create_data_and_create_playlist_does_not_wedge_the_next_run() {
    let mut writer = ArchiveWriter::new(config(ArchiveContainer::Ts, 2, periods(10))).expect("w");
    let mut store = MemStore::default();
    store.data.insert(0, vec![0; 100]);
    store.playlists.insert(0, vec![0; 50]);
    store.data.insert(1, vec![9; 40]);
    writer.seed(&store.seed_files()).expect("seed");
    let mut targets = Vec::new();
    for q in 0..6 {
        let plan = plan(
            writer
                .on_item(Timestamp::ZERO, None, &seg(q, 2))
                .expect("on_item"),
        );
        if let Some(target) = create_data_target(&plan.ops) {
            targets.push(target);
        }
        let outcome = store.execute(&plan.ops);
        assert_eq!(outcome, PlanOutcome::Done, "segment {q}");
        writer.complete(plan.id, outcome).expect("complete");
        assert!(writer.last_input_consumed(), "segment {q}");
    }
    assert_eq!(targets, vec![2]);
    assert_eq!(store.data[&1], vec![9; 40]);
}

#[test]
fn an_existing_media_file_consumes_its_number_and_the_retry_takes_the_next() {
    let mut writer = ArchiveWriter::new(config(ArchiveContainer::Ts, 2, periods(10))).expect("w");
    let mut store = MemStore::default();
    store.data.insert(0, vec![7; 10]);
    let first = run(&mut writer, &mut store, &seg(0, 2));
    assert_eq!(create_data_target(&first), Some(0));
    assert!(!writer.last_input_consumed());
    let retry = run(&mut writer, &mut store, &seg(0, 2));
    assert_eq!(create_data_target(&retry), Some(1));
    assert!(writer.last_input_consumed());
    assert_eq!(store.data[&0], vec![7; 10]);
    assert_eq!(store.listed_seqs(ArchiveContainer::Ts), vec![(1, 0)]);
}

#[test]
fn an_existing_playlist_consumes_its_number_and_the_retry_takes_the_next() {
    let mut writer = ArchiveWriter::new(config(ArchiveContainer::Ts, 2, periods(10))).expect("w");
    let mut store = MemStore::default();
    store.playlists.insert(0, b"foreign".to_vec());
    let first = run(&mut writer, &mut store, &seg(0, 2));
    assert_eq!(create_data_target(&first), Some(0));
    assert!(!writer.last_input_consumed());
    let retry = run(&mut writer, &mut store, &seg(0, 2));
    assert_eq!(create_data_target(&retry), Some(1));
    assert!(writer.last_input_consumed());
    assert_eq!(store.playlists[&0], b"foreign".to_vec());
}

#[test]
fn an_existing_file_at_the_last_number_exhausts_the_range_instead_of_looping() {
    let mut writer = ArchiveWriter::new(config(ArchiveContainer::Ts, 2, periods(10))).expect("w");
    writer
        .seed(&[SeedFile::new(
            ArchiveFile::Playlist(PeriodNum::new(u32::MAX - 2)),
            10,
        )])
        .expect("seed");
    let mut store = MemStore::default();
    store.data.insert(u32::MAX - 1, Vec::new());
    let first = run(&mut writer, &mut store, &seg(0, 2));
    assert_eq!(create_data_target(&first), Some(u32::MAX - 1));
    assert!(!writer.last_input_consumed());
    assert!(matches!(
        writer.on_item(Timestamp::ZERO, None, &seg(0, 2)),
        Err(ArchiveError::PeriodNumbersExhausted)
    ));
}

#[test]
fn seeded_orphan_media_counts_toward_the_byte_cap_and_is_evicted() {
    let mut writer =
        ArchiveWriter::new(config(ArchiveContainer::Ts, 2, bytes_limit(1000))).expect("writer");
    writer
        .seed(&[
            SeedFile::new(ArchiveFile::Playlist(PeriodNum::new(0)), 50),
            SeedFile::new(ArchiveFile::Media(PeriodNum::new(0)), 900),
            SeedFile::new(ArchiveFile::Media(PeriodNum::new(7)), 900),
        ])
        .expect("seed");
    let mut store = MemStore::default();
    store.data.insert(0, vec![0; 900]);
    store.playlists.insert(0, vec![0; 50]);
    store.data.insert(7, vec![0; 900]);
    let first = run(&mut writer, &mut store, &seg(0, 2));
    assert_eq!(create_data_target(&first), Some(8));
    assert_eq!(removed(&first), vec!["p0.m3u8", "p0.media"]);
    let second = run(&mut writer, &mut store, &seg(1, 2));
    assert_eq!(removed(&second), vec!["p7.m3u8", "p7.media"]);
    assert_eq!(store.data.keys().copied().collect::<Vec<_>>(), vec![8]);
    let evicted: Vec<u32> = events(&mut writer)
        .iter()
        .filter_map(|event| match event {
            WriterEvent::Evicted { period } => Some(period.value()),
            _ => None,
        })
        .collect();
    assert_eq!(evicted, vec![0, 7]);
}

#[test]
fn a_period_orphaned_by_a_failed_roll_is_evicted_like_any_period() {
    let mut writer = ArchiveWriter::new(config(ArchiveContainer::Ts, 2, periods(1))).expect("w");
    let mut store = MemStore::default();
    run(&mut writer, &mut store, &seg(0, 2));
    run(&mut writer, &mut store, &seg(1, 2));
    let roll = plan(
        writer
            .on_item(Timestamp::ZERO, None, &seg(2, 5))
            .expect("roll"),
    );
    let create_playlist = roll
        .ops
        .iter()
        .position(|op| matches!(op, StoreOp::CreatePlaylist { .. }))
        .expect("create playlist");
    let outcome = store.execute(&roll.ops[..create_playlist]);
    assert_eq!(outcome, PlanOutcome::Done);
    writer
        .complete(
            roll.id,
            PlanOutcome::Failed {
                at_op: create_playlist,
                failure: StoreFailure::Io(ErrorKind::Other),
            },
        )
        .expect("complete");
    assert!(!writer.last_input_consumed());
    assert_eq!(store.data[&1].len(), 102);
    let retry = run(&mut writer, &mut store, &seg(2, 5));
    assert_eq!(create_data_target(&retry), Some(2));
    run(&mut writer, &mut store, &seg(3, 5));
    let roll_again = run(&mut writer, &mut store, &seg(4, 9));
    assert_eq!(create_data_target(&roll_again), Some(3));
    let mut all_removed = removed(&retry);
    all_removed.extend(removed(&roll_again));
    assert_eq!(
        all_removed,
        vec!["p0.m3u8", "p0.media", "p1.m3u8", "p1.media"]
    );
    assert_eq!(store.data.keys().copied().collect::<Vec<_>>(), vec![2, 3]);
}

#[test]
fn a_crash_at_every_op_of_a_roll_leaves_a_readable_archive_and_a_working_restart() {
    for crash in 0..12usize {
        let mut writer =
            ArchiveWriter::new(config(ArchiveContainer::Fmp4, 2, periods(10))).expect("w");
        writer
            .on_inits(&[
                InitGeneration::new(SegmentSeq::new(0), Bytes::from_static(b"INIT1")),
                InitGeneration::new(SegmentSeq::new(2), Bytes::from_static(b"INIT2")),
            ])
            .expect("inits");
        let mut store = MemStore::default();
        run(&mut writer, &mut store, &seg(0, 2));
        run(&mut writer, &mut store, &seg(1, 2));
        let roll = plan(
            writer
                .on_item(Timestamp::ZERO, None, &seg(2, 2))
                .expect("roll"),
        );
        if crash >= roll.ops.len() {
            assert_eq!(roll.ops.len(), 9);
            break;
        }
        store.crash_after(&roll.ops, crash);
        let listed = store.listed_seqs(ArchiveContainer::Fmp4);
        let mut seqs: Vec<u32> = listed.iter().map(|(_, seq)| *seq).collect();
        seqs.dedup();
        assert_eq!(
            seqs.len(),
            listed.len(),
            "crash after op {crash}: {listed:?}"
        );

        let mut restarted =
            ArchiveWriter::new(config(ArchiveContainer::Fmp4, 2, periods(10))).expect("w");
        restarted.seed(&store.seed_files()).expect("seed");
        restarted
            .on_inits(&[InitGeneration::new(
                SegmentSeq::new(3),
                Bytes::from_static(b"INIT2"),
            )])
            .expect("inits");
        let next = plan(
            restarted
                .on_item(Timestamp::ZERO, None, &seg(3, 2))
                .expect("restart"),
        );
        let outcome = store.execute(&next.ops);
        assert_eq!(outcome, PlanOutcome::Done, "crash after op {crash}");
        restarted.complete(next.id, outcome).expect("complete");
        assert!(restarted.last_input_consumed(), "crash after op {crash}");
        let listed = store.listed_seqs(ArchiveContainer::Fmp4);
        assert_eq!(listed.last().map(|(_, seq)| *seq), Some(3));
    }
}

#[test]
fn a_renamed_playlist_whose_directory_sync_failed_is_not_archived_twice() {
    let mut writer = ArchiveWriter::new(config(ArchiveContainer::Ts, 2, periods(10))).expect("w");
    let mut store = MemStore::default();
    run(&mut writer, &mut store, &seg(0, 2));
    run(&mut writer, &mut store, &seg(1, 2));
    let roll = plan(
        writer
            .on_item(Timestamp::ZERO, None, &seg(2, 5))
            .expect("roll"),
    );
    let create_playlist = roll
        .ops
        .iter()
        .position(|op| matches!(op, StoreOp::CreatePlaylist { .. }))
        .expect("create playlist");
    store.crash_after(&roll.ops, create_playlist);
    let _ = events(&mut writer);
    writer
        .complete(
            roll.id,
            PlanOutcome::Failed {
                at_op: create_playlist,
                failure: StoreFailure::NotDurable(ErrorKind::Other),
            },
        )
        .expect("complete");
    assert!(writer.last_input_consumed());
    assert!(events(&mut writer).contains(&WriterEvent::NotDurable {
        op: StoreOpKind::CreatePlaylist,
        period: Some(PeriodNum::new(1)),
        kind: ErrorKind::Other,
    }));
    for q in 3..6 {
        run(&mut writer, &mut store, &seg(q, 5));
    }
    assert_eq!(
        store.listed_seqs(ArchiveContainer::Ts),
        vec![(0, 0), (0, 1), (1, 2), (1, 3), (1, 4), (1, 5)]
    );
    assert!(matches!(
        writer
            .on_item(Timestamp::ZERO, None, &seg(6, 2))
            .expect("next"),
        Step::Execute(_)
    ));
}

#[cfg(feature = "fs")]
#[test]
fn a_sequence_claimed_by_two_periods_is_scanned_once_from_the_earlier_period() {
    use dvr_runtime::fs::FsArchive;
    use dvr_runtime::read::SeqConflict;
    use dvr_runtime::{ArchiveName, layout};

    let mut writer = ArchiveWriter::new(config(ArchiveContainer::Ts, 2, periods(10))).expect("w");
    let mut store = MemStore::default();
    run(&mut writer, &mut store, &seg(0, 2));
    run(&mut writer, &mut store, &seg(1, 2));
    let roll = plan(
        writer
            .on_item(Timestamp::ZERO, None, &seg(2, 5))
            .expect("roll"),
    );
    let create_playlist = roll
        .ops
        .iter()
        .position(|op| matches!(op, StoreOp::CreatePlaylist { .. }))
        .expect("create playlist");
    store.crash_after(&roll.ops, create_playlist);
    writer
        .complete(
            roll.id,
            PlanOutcome::Failed {
                at_op: create_playlist,
                failure: StoreFailure::Io(ErrorKind::Other),
            },
        )
        .expect("complete");
    run(&mut writer, &mut store, &seg(2, 5));
    run(&mut writer, &mut store, &seg(3, 5));

    let root = crate::support::temp_dir("seq-conflict");
    let name = ArchiveName::new("route").expect("name");
    let dir = layout::route_dir(&root, &name);
    std::fs::create_dir_all(&dir).expect("dir");
    for (period, bytes) in &store.data {
        let file = layout::media_file_name(PeriodNum::new(*period), ArchiveContainer::Ts);
        std::fs::write(dir.join(file), bytes).expect("media");
    }
    for (period, text) in &store.playlists {
        let file = layout::playlist_file_name(PeriodNum::new(*period));
        std::fs::write(dir.join(file), text).expect("playlist");
    }
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Ts);
    let scan = archive.scan();
    let listed: Vec<(u32, u32)> = scan
        .segments
        .iter()
        .map(|s| (s.period.value(), s.seq.value()))
        .collect();
    assert_eq!(listed, vec![(0, 0), (0, 1), (1, 2), (2, 3)]);
    assert_eq!(
        scan.conflicts,
        vec![SeqConflict::new(
            SegmentSeq::new(2),
            PeriodNum::new(1),
            PeriodNum::new(2)
        )]
    );
    for segment in &scan.segments {
        let bytes = archive.read(segment).expect("read");
        assert_eq!(bytes.len(), 100 + segment.seq.value() as usize);
        assert!(bytes.iter().all(|b| u32::from(*b) == segment.seq.value()));
    }
    crate::support::cleanup(&root);
}
