//! The fs adapter: seeding, the archive floor, cache bounds and read errors
//! (8.1 seed/floor rows, #3678, #3685).

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use dvr_runtime::fs::{FsArchive, FsError, FsRecorder};
use dvr_runtime::read::ReadError;
use dvr_runtime::recover::RecoveredBox;
use dvr_runtime::writer::InitGeneration;
use dvr_runtime::{ArchiveContainer, ArchiveName, RetentionLimits};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, Trunk, TrunkConfig};

use crate::support::{cleanup, config, periods, temp_dir};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("nonzero")
}

fn trunk() -> Arc<Trunk> {
    Trunk::new(
        TrunkConfig::default()
            .with_segment_capacity(nz(16))
            .with_timed_capacity(nz(4))
            .with_sparse_capacity(nz(4))
            .with_event_capacity(nz(4))
            .with_part_capacity(nz(4)),
    )
}

fn init() -> Bytes {
    Bytes::from(vec![0x49u8; 812])
}

fn record_run(
    root: &std::path::Path,
    name: &str,
    retention: RetentionLimits,
    offset: u64,
    segments: &[(u32, u8)],
) {
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let config = config(ArchiveContainer::Fmp4, 2, retention).with_seq_offset(offset);
    let mut recorder = FsRecorder::new(root, ArchiveName::new(name).expect("name"), config, &trunk)
        .expect("recorder");
    recorder
        .poll(
            Timestamp::ZERO,
            |_| None,
            &[InitGeneration::new(SegmentSeq::new(0), init())],
        )
        .expect("init");
    for &(seq, fill) in segments {
        let entry = SegmentEntry::new(
            Bytes::from(vec![fill; 100]),
            SegmentSeq::new(seq),
            Duration::from_secs(2),
            Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
            transmux::SegmentMeta {
                discontinuous: false,
            },
        );
        writer.publish_segment(entry).expect("publish");
    }
    recorder
        .poll(Timestamp::ZERO, |_| None, &[])
        .expect("record");
}

#[test]
fn a_later_recorder_leaves_earlier_periods_untouched() {
    let root = temp_dir("seed");
    record_run(
        &root,
        "run",
        periods(10),
        0,
        &[(0, 0xA0), (1, 0xA1), (2, 0xA2)],
    );
    let dir = root.join("run");
    let p0_media = std::fs::read(dir.join("p0.m4s")).expect("p0 media");
    let p0_playlist = std::fs::read(dir.join("p0.m3u8")).expect("p0 playlist");

    record_run(&root, "run", periods(10), 3, &[(0, 0xB0), (1, 0xB1)]);
    assert_eq!(
        std::fs::read(dir.join("p0.m4s")).expect("p0 media"),
        p0_media
    );
    assert_eq!(
        std::fs::read(dir.join("p0.m3u8")).expect("p0 playlist"),
        p0_playlist
    );
    let p1 = std::fs::read(dir.join("p1.m4s")).expect("p1");
    assert!(p1.starts_with(&init()));
    let mut tail = vec![0xB0u8; 100];
    tail.extend_from_slice(&[0xB1u8; 100]);
    assert_eq!(&p1[init().len()..], tail.as_slice());

    let mut archive = FsArchive::open(
        &root,
        &ArchiveName::new("run").expect("name"),
        ArchiveContainer::Fmp4,
    );
    let scan = archive.scan();
    assert!(scan.unreadable.is_empty());
    assert_eq!(
        scan.segments
            .iter()
            .map(|s| s.seq.value())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert!(
        scan.segments[3].discontinuous,
        "a new run does not continue the timeline"
    );
    assert!(!scan.segments[4].discontinuous);
    let found = archive.find(SegmentSeq::new(3)).expect("seq 3");
    assert_eq!(found.period.value(), 1);
    assert_eq!(
        archive.read(&found).expect("read"),
        Bytes::from(vec![0xB0u8; 100])
    );
    cleanup(&root);
}

#[test]
fn retention_covers_periods_left_by_earlier_runs() {
    let root = temp_dir("seed-retention");
    for (n, fill) in [(0u32, 1u8), (1, 2), (2, 3), (3, 4)] {
        record_run(&root, "ret", periods(2), u64::from(n), &[(0, fill)]);
    }
    let dir = root.join("ret");
    assert!(!dir.join("p0.m4s").exists(), "p0 media is evicted");
    assert!(!dir.join("p0.m3u8").exists(), "p0 playlist is evicted");
    assert!(dir.join("p3.m4s").exists());
    cleanup(&root);
}

#[test]
fn archive_floor_is_the_highest_archived_sequence() {
    let root = temp_dir("floor");
    let name = ArchiveName::new("floor").expect("name");
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    assert_eq!(archive.seq_floor(), SegmentSeq::new(0), "no archive yet");
    record_run(&root, "floor", periods(10), 0, &[(0, 1), (1, 2), (2, 3)]);
    record_run(&root, "floor", periods(10), 3, &[(0, 4), (1, 5)]);
    assert_eq!(archive.seq_floor(), SegmentSeq::new(4));
    cleanup(&root);
}

#[test]
fn a_non_not_found_listing_error_is_surfaced() {
    if std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .is_some_and(|uid| uid.trim() == "0")
    {
        eprintln!("SKIP: running as root, so a 000 directory is still readable");
        return;
    }
    let root = temp_dir("chmod");
    let dir = root.join("locked");
    std::fs::create_dir_all(&dir).expect("dir");
    let mut perms = std::fs::metadata(&dir).expect("meta").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
    std::fs::set_permissions(&dir, perms).expect("chmod");
    let trunk = trunk();
    let result = FsRecorder::new(
        &root,
        ArchiveName::new("locked").expect("name"),
        config(ArchiveContainer::Fmp4, 2, periods(2)),
        &trunk,
    );
    let mut perms = std::fs::metadata(&dir).expect("meta").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&dir, perms).expect("restore");
    match result {
        Err(FsError::Io { .. }) => {}
        Err(other) => panic!("expected an I/O error, got {other}"),
        Ok(_) => panic!("a 000 directory must not read as an empty archive"),
    }
    cleanup(&root);
}

#[test]
fn the_cache_is_invalidated_when_a_playlist_changes() {
    let root = temp_dir("cache-change");
    let name = ArchiveName::new("cache").expect("name");
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let mut config = config(ArchiveContainer::Fmp4, 2, periods(10));
    config.period_duration = Some(Duration::from_secs(86400));
    let mut recorder = FsRecorder::new(
        &root,
        ArchiveName::new("cache").expect("name"),
        config,
        &trunk,
    )
    .expect("recorder");
    recorder
        .poll(
            Timestamp::ZERO,
            |_| None,
            &[InitGeneration::new(SegmentSeq::new(0), init())],
        )
        .expect("init");
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    for seq in 0..2u32 {
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(vec![0x11u8; 100]),
                SegmentSeq::new(seq),
                Duration::from_secs(2),
                Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
                transmux::SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
        recorder
            .poll(Timestamp::ZERO, |_| None, &[])
            .expect("record");
        assert_eq!(archive.scan().segments.len(), seq as usize + 1);
        assert_eq!(archive.cached_periods(), 1);
    }
    cleanup(&root);
}

#[test]
fn the_cache_invalidates_a_same_length_rewrite_with_a_new_inode() {
    let root = temp_dir("cache-inode");
    let name = ArchiveName::new("cache").expect("name");
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let mut config = config(ArchiveContainer::Fmp4, 2, periods(10));
    config.period_duration = Some(Duration::from_secs(86400));
    let mut recorder = FsRecorder::new(
        &root,
        ArchiveName::new("cache").expect("name"),
        config,
        &trunk,
    )
    .expect("recorder");
    recorder
        .poll(
            Timestamp::ZERO,
            |_| None,
            &[InitGeneration::new(SegmentSeq::new(0), init())],
        )
        .expect("init");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(vec![0x11u8; 100]),
            SegmentSeq::new(0),
            Duration::from_secs(2),
            Timestamp::from_nanos(2_000_000_000),
            transmux::SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    recorder
        .poll(Timestamp::ZERO, |_| None, &[])
        .expect("record");

    let playlist = root.join("cache").join("p0.m3u8");
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    assert_eq!(
        archive.scan().segments[0].start,
        Timestamp::from_nanos(2_000_000_000)
    );
    let before = std::fs::read_to_string(&playlist).expect("playlist");
    let after = before.replace("DTS=2000000000", "DTS=2000001000");
    assert_eq!(
        before.len(),
        after.len(),
        "the rewrite must keep the length"
    );
    let mtime = std::fs::metadata(&playlist)
        .expect("meta")
        .modified()
        .expect("mtime");
    let replacement = root.join("cache").join("p0.m3u8.new");
    std::fs::write(&replacement, after).expect("write");
    std::fs::rename(&replacement, &playlist).expect("rename");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&playlist)
        .expect("open");
    file.set_modified(mtime).expect("restore mtime");
    drop(file);
    assert_eq!(
        archive.scan().segments[0].start,
        Timestamp::from_nanos(2_000_001_000),
        "a same-length rewrite with the same mtime is caught by the inode"
    );
    cleanup(&root);
}

#[test]
fn each_archive_has_its_own_cache() {
    let first = temp_dir("cache-first");
    let second = temp_dir("cache-second");
    record_run(&first, "a", periods(10), 0, &[(0, 1)]);
    record_run(&second, "b", periods(10), 0, &[(0, 2)]);
    let mut a = FsArchive::open(
        &first,
        &ArchiveName::new("a").expect("name"),
        ArchiveContainer::Fmp4,
    );
    let mut b = FsArchive::open(
        &second,
        &ArchiveName::new("b").expect("name"),
        ArchiveContainer::Fmp4,
    );
    assert_eq!(a.scan().segments.len(), 1);
    assert_eq!(a.cached_periods(), 1);
    assert_eq!(
        b.cached_periods(),
        0,
        "one archive's cache never covers another"
    );
    assert_eq!(b.scan().segments.len(), 1);
    assert_eq!(b.cached_periods(), 1);
    assert_eq!(a.cached_periods(), 1);
    drop(a);
    let a = FsArchive::open(
        &first,
        &ArchiveName::new("a").expect("name"),
        ArchiveContainer::Fmp4,
    );
    assert_eq!(
        a.cached_periods(),
        0,
        "dropping the archive releases its cache"
    );
    cleanup(&first);
    cleanup(&second);
}

#[test]
fn reading_a_gone_or_short_period_is_typed() {
    let root = temp_dir("gone");
    let name = ArchiveName::new("gone").expect("name");
    record_run(&root, "gone", periods(10), 0, &[(0, 1)]);
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    let segment = archive.find(SegmentSeq::new(0)).expect("segment");
    std::fs::remove_file(root.join("gone").join("p0.m4s")).expect("remove media");
    assert!(matches!(
        archive.read(&segment),
        Err(ReadError::Gone { .. })
    ));
    std::fs::write(root.join("gone").join("p0.m4s"), [0u8; 4]).expect("short media");
    assert!(matches!(
        archive.read(&segment),
        Err(ReadError::RangeBeyondFile { file_len: 4, .. })
    ));
    cleanup(&root);
}

#[test]
fn recover_spans_walks_a_recorded_period() {
    let root = temp_dir("recover-fs");
    let name = ArchiveName::new("rec").expect("name");
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let mut config = config(ArchiveContainer::Fmp4, 2, periods(10));
    config.period_duration = Some(Duration::from_secs(86400));
    let mut recorder = FsRecorder::new(
        &root,
        ArchiveName::new("rec").expect("name"),
        config,
        &trunk,
    )
    .expect("recorder");
    let ftyp = boxed(b"ftyp", b"isom0000");
    let moov = boxed(b"moov", &[0x11; 16]);
    let init: Vec<u8> = [ftyp.clone(), moov.clone()].concat();
    recorder
        .poll(
            Timestamp::ZERO,
            |_| None,
            &[InitGeneration::new(
                SegmentSeq::new(0),
                Bytes::from(init.clone()),
            )],
        )
        .expect("init");
    let pair = [boxed(b"moof", &[0x22; 8]), boxed(b"mdat", &[0x33; 32])].concat();
    for seq in 0..2u32 {
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(pair.clone()),
                SegmentSeq::new(seq),
                Duration::from_secs(2),
                Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
                transmux::SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    recorder
        .poll(Timestamp::ZERO, |_| None, &[])
        .expect("record");

    let archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    let recovered = archive
        .recover_spans(dvr_runtime::PeriodNum::new(0))
        .expect("recover");
    assert_eq!(recovered.len(), 3);
    assert!(
        matches!(recovered[0], RecoveredBox::Init(span) if span.offset == 0 && span.len.get() == init.len() as u64)
    );
    assert!(
        matches!(recovered[1], RecoveredBox::Fragment(span) if span.offset == init.len() as u64 && span.len.get() == pair.len() as u64)
    );
    assert!(
        matches!(recovered[2], RecoveredBox::Fragment(span) if span.offset == (init.len() + pair.len()) as u64)
    );
    cleanup(&root);
}

fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8 + payload.len()).expect("size");
    let mut bytes = size.to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn a_broken_period_is_reported_and_the_others_still_read() {
    let root = temp_dir("unreadable");
    let name = ArchiveName::new("broken").expect("name");
    record_run(&root, "broken", periods(10), 0, &[(0, 1), (1, 2)]);
    let dir = root.join("broken");
    std::fs::write(dir.join("p1.m3u8"), b"not a playlist\n").expect("write junk");
    std::fs::write(dir.join("p1.m4s"), [0u8; 4]).expect("write junk media");
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    let scan = archive.scan();
    assert_eq!(scan.segments.len(), 2);
    assert_eq!(scan.unreadable.len(), 1);
    assert_eq!(scan.unreadable[0].0, dvr_runtime::PeriodNum::new(1));
    assert!(matches!(
        scan.unreadable[0].1,
        dvr_runtime::read::PlaylistError::Hls(_)
    ));
    cleanup(&root);
}
