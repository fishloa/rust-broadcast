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
use dvr_runtime::{ArchiveConfig, ArchiveContainer, ArchiveName, RetentionLimits};
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

/// Open a recorder on a directory a previous run used, retrying a `Locked`
/// refusal for a bounded window: a released lock file can still be shared for
/// a moment by a process the host spawned (`FsRecorder::succeed` documents
/// the same window), so a host that restarts a route retries rather than
/// failing. `Locked` stays an error past the window.
fn recorder_of(
    root: &std::path::Path,
    name: &str,
    config: ArchiveConfig,
    trunk: &Arc<Trunk>,
) -> FsRecorder {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match FsRecorder::new(
            root,
            ArchiveName::new(name).expect("name"),
            config.clone(),
            trunk,
        ) {
            Ok(recorder) => return recorder,
            Err(FsError::Locked { .. }) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(other) => panic!("recorder: {other}"),
        }
    }
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
    let mut recorder = recorder_of(root, name, config, &trunk);
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
    let scan = archive.scan().expect("scan");
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
    let found = archive
        .find(SegmentSeq::new(3))
        .expect("listing")
        .expect("seq 3");
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
    assert_eq!(archive.seq_floor().expect("floor"), None, "no archive yet");
    record_run(&root, "floor", periods(10), 0, &[(0, 1), (1, 2), (2, 3)]);
    record_run(&root, "floor", periods(10), 3, &[(0, 4), (1, 5)]);
    assert_eq!(
        archive.seq_floor().expect("floor"),
        Some(SegmentSeq::new(4))
    );
    cleanup(&root);
}

fn running_as_root() -> bool {
    let root = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .is_some_and(|uid| uid.trim() == "0");
    if root {
        eprintln!("SKIP: running as root, so a 000 path is still readable");
    }
    root
}

fn set_mode(path: &std::path::Path, mode: u32) {
    let mut perms = std::fs::metadata(path).expect("meta").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, mode);
    std::fs::set_permissions(path, perms).expect("chmod");
}

#[test]
fn an_unlistable_archive_is_an_error_not_an_empty_floor() {
    if running_as_root() {
        return;
    }
    let root = temp_dir("floor-chmod");
    record_run(&root, "route", periods(10), 0, &[(0, 1), (1, 2), (2, 3)]);
    let name = ArchiveName::new("route").expect("name");
    let dir = root.join("route");
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    assert_eq!(
        archive.seq_floor().expect("floor"),
        Some(SegmentSeq::new(2))
    );
    set_mode(&dir, 0o000);
    let floor = archive.seq_floor();
    let scan = archive.scan();
    let find = archive.find(SegmentSeq::new(0));
    set_mode(&dir, 0o755);
    assert!(
        matches!(floor, Err(ReadError::Io { .. })),
        "seq_floor: {floor:?}"
    );
    assert!(matches!(scan, Err(ReadError::Io { .. })), "scan: {scan:?}");
    assert!(matches!(find, Err(ReadError::Io { .. })), "find: {find:?}");
    cleanup(&root);
}

#[test]
fn an_unreadable_period_playlist_fails_the_floor() {
    if running_as_root() {
        return;
    }
    let root = temp_dir("floor-playlist-chmod");
    record_run(&root, "route", periods(10), 0, &[(0, 1), (1, 2), (2, 3)]);
    let name = ArchiveName::new("route").expect("name");
    let playlist = root.join("route").join("p0.m3u8");
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    set_mode(&playlist, 0o000);
    let floor = archive.seq_floor();
    set_mode(&playlist, 0o644);
    assert!(
        matches!(
            floor,
            Err(ReadError::Playlist(
                dvr_runtime::read::PlaylistError::Io { .. }
            ))
        ),
        "{floor:?}"
    );
    cleanup(&root);
}

#[test]
fn a_non_not_found_listing_error_is_surfaced() {
    if running_as_root() {
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
        assert_eq!(
            archive.scan().expect("scan").segments.len(),
            seq as usize + 1
        );
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
        archive.scan().expect("scan").segments[0].start,
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
        archive.scan().expect("scan").segments[0].start,
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
    assert_eq!(a.scan().expect("scan").segments.len(), 1);
    assert_eq!(a.cached_periods(), 1);
    assert_eq!(
        b.cached_periods(),
        0,
        "one archive's cache never covers another"
    );
    assert_eq!(b.scan().expect("scan").segments.len(), 1);
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
    let segment = archive
        .find(SegmentSeq::new(0))
        .expect("listing")
        .expect("segment");
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
    let scan = archive.scan().expect("scan");
    assert_eq!(scan.segments.len(), 2);
    assert_eq!(scan.unreadable.len(), 1);
    assert_eq!(scan.unreadable[0].0, dvr_runtime::PeriodNum::new(1));
    assert!(matches!(
        scan.unreadable[0].1,
        dvr_runtime::read::PlaylistError::Hls(_)
    ));
    cleanup(&root);
}

#[test]
fn a_restart_counts_each_seeded_periods_real_size() {
    let root = temp_dir("seed-bytes");
    let name = ArchiveName::new("sized").expect("name");
    let cfg = ArchiveConfig::new(
        ArchiveContainer::Ts,
        Duration::from_secs(2),
        dvr_runtime::RetentionLimits::new()
            .with_bytes(std::num::NonZeroU64::new(2000).expect("nonzero")),
    )
    .with_period_duration(Some(Duration::from_secs(3600)));
    for fill in [0x11u8, 0x22] {
        let trunk = trunk();
        let writer = trunk.segment_writer().expect("writer");
        let mut recorder = FsRecorder::new(
            &root,
            ArchiveName::new("sized").expect("name"),
            cfg.clone(),
            &trunk,
        )
        .expect("recorder");
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(vec![fill; 1500]),
                SegmentSeq::new(0),
                Duration::from_secs(2),
                Timestamp::ZERO,
                transmux::SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
        recorder
            .poll(Timestamp::ZERO, |_| None, &[])
            .expect("record");
    }
    let dir = root.join("sized");
    assert!(
        !dir.join("p0.ts").exists(),
        "the seeded period's real size must count toward the cap"
    );
    assert!(!dir.join("p0.m3u8").exists());
    assert!(dir.join("p1.ts").exists());
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Ts);
    let scan = archive.scan().expect("scan");
    assert_eq!(scan.segments.len(), 1);
    assert_eq!(scan.segments[0].period.value(), 1);
    cleanup(&root);
}

fn recorder_for(root: &std::path::Path, name: &str) -> Result<FsRecorder, FsError> {
    FsRecorder::new(
        root,
        ArchiveName::new(name).expect("name"),
        config(ArchiveContainer::Fmp4, 2, periods(10)),
        &trunk(),
    )
}

#[test]
fn a_second_recorder_on_one_directory_is_refused() {
    let root = temp_dir("lock");
    let first = recorder_for(&root, "route").expect("first");
    match recorder_for(&root, "route") {
        Err(FsError::Locked { path }) => assert!(path.ends_with(".lock")),
        other => panic!("expected Locked, got {:?}", other.map(|_| ())),
    }
    drop(first);
    cleanup(&root);
}

#[test]
fn dropping_the_recorder_releases_the_lock() {
    let root = temp_dir("lock-release");
    let config = config(ArchiveContainer::Fmp4, 2, periods(10));
    let trunk = trunk();
    drop(recorder_of(&root, "route", config.clone(), &trunk));
    drop(recorder_of(&root, "route", config, &trunk));
    cleanup(&root);
}

#[test]
fn the_lock_file_is_never_scanned_listed_or_evicted() {
    let root = temp_dir("lock-ignored");
    let dir = root.join("route");
    record_run(&root, "route", periods(1), 0, &[(0, 1)]);
    record_run(&root, "route", periods(1), 1, &[(0, 2)]);
    record_run(&root, "route", periods(1), 2, &[(0, 3)]);
    assert!(dir.join(".lock").exists(), "retention never removes it");
    assert!(!dir.join("p0.m4s").exists(), "retention still evicts");
    let mut archive = FsArchive::open(
        &root,
        &ArchiveName::new("route").expect("name"),
        ArchiveContainer::Fmp4,
    );
    let scan = archive.scan().expect("scan");
    assert!(scan.unreadable.is_empty());
    assert!(scan.conflicts.is_empty());
    assert!(dvr_runtime::layout::classify(".lock", ArchiveContainer::Fmp4).is_none());
    cleanup(&root);
}

#[test]
fn a_restart_given_the_seq_floor_produces_no_conflicts() {
    let root = temp_dir("floor-ok");
    record_run(&root, "route", periods(10), 0, &[(0, 1), (1, 2), (2, 3)]);
    let name = ArchiveName::new("route").expect("name");
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    let floor = archive.seq_floor().expect("floor").expect("archived");
    let offset = u64::from(floor.value()) + 1;
    record_run(&root, "route", periods(10), offset, &[(0, 4), (1, 5)]);
    let scan = archive.scan().expect("scan");
    assert!(scan.conflicts.is_empty());
    assert_eq!(
        scan.segments
            .iter()
            .map(|s| s.seq.value())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    cleanup(&root);
}

#[test]
fn a_restart_without_the_seq_floor_reports_the_conflict() {
    let root = temp_dir("floor-missing");
    record_run(&root, "route", periods(10), 0, &[(0, 1), (1, 2), (2, 3)]);
    record_run(&root, "route", periods(10), 0, &[(0, 4), (1, 5)]);
    let name = ArchiveName::new("route").expect("name");
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Fmp4);
    let scan = archive.scan().expect("scan");
    assert_eq!(scan.segments.len(), 3, "the later run is hidden");
    let conflicts: Vec<u32> = scan.conflicts.iter().map(|c| c.seq.value()).collect();
    assert_eq!(conflicts, vec![0, 1]);
    assert!(
        scan.conflicts
            .iter()
            .all(|c| c.dropped.value() > c.kept.value())
    );
    cleanup(&root);
}

#[test]
fn a_stale_temp_playlist_is_removed_reported_and_never_an_archive_file() {
    use dvr_runtime::PeriodNum;
    use dvr_runtime::writer::WriterEvent;

    let root = temp_dir("stale-tmp");
    let dir = root.join("route");
    record_run(&root, "route", periods(10), 0, &[(0, 1)]);
    std::fs::write(dir.join(".p5.m3u8.tmp"), b"#EXTM3U\n").expect("tmp");
    assert!(dvr_runtime::layout::classify(".p5.m3u8.tmp", ArchiveContainer::Fmp4).is_none());
    let mut recorder = recorder_for(&root, "route").expect("recorder");
    assert!(!dir.join(".p5.m3u8.tmp").exists());
    assert_eq!(
        recorder.poll_event(),
        Some(WriterEvent::StaleTempPlaylistRemoved {
            period: PeriodNum::new(5)
        })
    );
    drop(recorder);
    let mut archive = FsArchive::open(
        &root,
        &ArchiveName::new("route").expect("name"),
        ArchiveContainer::Fmp4,
    );
    let scan = archive.scan().expect("scan");
    assert!(scan.unreadable.is_empty());
    assert_eq!(scan.segments.len(), 1);
    cleanup(&root);
}

/// A host that pins its cursor before the previous recorder of the archive
/// has released the directory keeps the cursor through the `Locked` refusal
/// and, once it can create the recorder, archives the segment that closed in
/// between. (A child process another test forks can briefly share the
/// dropped lock's file description, so the second create retries `Locked`
/// as a host does.)
#[test]
fn a_held_cursor_survives_a_locked_directory_and_records_what_closed_meanwhile() {
    let root = temp_dir("held-cursor");
    let name = || ArchiveName::new("held").expect("name");
    let config = || config(ArchiveContainer::Fmp4, 2, periods(10));
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let first = FsRecorder::new(&root, name(), config(), &trunk).expect("first recorder");
    let mut cursor = Some(trunk.pin_segments(config().overrun));
    assert!(matches!(
        FsRecorder::with_cursor(&root, name(), config(), &trunk, &mut cursor),
        Err(FsError::Locked { .. })
    ));
    assert!(cursor.is_some(), "a refused create keeps the cursor");
    drop(first);
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(vec![0x5Au8; 100]),
            SegmentSeq::new(1),
            Duration::from_secs(2),
            Timestamp::ZERO,
            transmux::SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    let mut attempts = 0;
    let mut second = loop {
        match FsRecorder::with_cursor(&root, name(), config(), &trunk, &mut cursor) {
            Ok(recorder) => break recorder,
            Err(FsError::Locked { .. }) if attempts < 100 => {
                assert!(cursor.is_some(), "a refused create keeps the cursor");
                attempts += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(other) => panic!("second recorder: {other}"),
        }
    };
    assert!(cursor.is_none(), "the recorder took the cursor");
    second
        .poll(
            Timestamp::ZERO,
            |_| None,
            &[InitGeneration::new(SegmentSeq::new(0), init())],
        )
        .expect("record");
    let mut archive = FsArchive::open(&root, &name(), ArchiveContainer::Fmp4);
    let found = archive
        .find(SegmentSeq::new(1))
        .expect("listing")
        .expect("seq 1 archived");
    assert_eq!(
        archive.read(&found).expect("read"),
        Bytes::from(vec![0x5Au8; 100])
    );
    cleanup(&root);
}

/// A programme change the host detected before the recorder existed, handed
/// over through `on_programme`, is recorded as the next segment's
/// `EXT-X-DATERANGE`; the same event again changes nothing.
#[test]
fn a_programme_handed_over_by_the_host_is_recorded() {
    let root = temp_dir("on-programme");
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let mut recorder = FsRecorder::new(
        &root,
        ArchiveName::new("prog").expect("name"),
        config(ArchiveContainer::Fmp4, 2, periods(10)).with_programme_service(0x0601),
        &trunk,
    )
    .expect("recorder");
    let mut tracker = dvb_si::epg::PresentEventTracker::new(0x0601);
    let change = crate::support::present_event(
        &mut tracker,
        0x0601,
        0x7857,
        "News",
        crate::support::ts_at(1_700_000_000),
        Duration::from_secs(1800),
    );
    recorder.on_programme(change.clone()).expect("apply");
    recorder.on_programme(change).expect("apply again");
    assert_eq!(
        recorder.writer().programme().map(|p| p.event.event_id),
        Some(0x7857)
    );
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(vec![0x11u8; 100]),
            SegmentSeq::new(1),
            Duration::from_secs(2),
            Timestamp::ZERO,
            transmux::SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    recorder
        .poll(
            Timestamp::ZERO,
            |_| Some(crate::support::ts_at(1_700_000_010)),
            &[InitGeneration::new(SegmentSeq::new(0), init())],
        )
        .expect("record");
    let playlist = std::fs::read_to_string(root.join("prog").join("p0.m3u8")).expect("playlist");
    assert_eq!(
        playlist.matches("#EXT-X-DATERANGE").count(),
        1,
        "{playlist}"
    );
    assert!(playlist.contains("X-DVB-EVENT-ID=30807"), "{playlist}");
    cleanup(&root);
}

/// `succeed` hands the directory lock from one run's recorder to the next
/// without releasing it: a third recorder is refused before, during and after
/// the hand-over, and the new run records into a new period.
#[test]
fn succeed_takes_the_lock_over_without_releasing_it() {
    let root = temp_dir("succeed");
    let name = || ArchiveName::new("succ").expect("name");
    let config = || config(ArchiveContainer::Fmp4, 2, periods(10));
    let refused = || {
        let other = trunk();
        matches!(
            FsRecorder::new(&root, name(), config(), &other),
            Err(FsError::Locked { .. })
        )
    };
    let first_trunk = trunk();
    let first = FsRecorder::new(&root, name(), config(), &first_trunk).expect("first");
    assert!(refused());
    let second_trunk = trunk();
    let writer = second_trunk.segment_writer().expect("writer");
    let mut cursor = Some(second_trunk.pin_segments(config().overrun));
    let mut second = first
        .succeed(config(), &second_trunk, &mut cursor)
        .expect("succeed");
    assert!(cursor.is_none(), "the successor took the cursor");
    assert!(refused(), "the lock was never released");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(vec![0x22u8; 100]),
            SegmentSeq::new(1),
            Duration::from_secs(2),
            Timestamp::ZERO,
            transmux::SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    second
        .poll(
            Timestamp::ZERO,
            |_| None,
            &[InitGeneration::new(SegmentSeq::new(0), init())],
        )
        .expect("record");
    let mut archive = FsArchive::open(&root, &name(), ArchiveContainer::Fmp4);
    let found = archive
        .find(SegmentSeq::new(1))
        .expect("listing")
        .expect("archived");
    assert_eq!(
        archive.read(&found).expect("read"),
        Bytes::from(vec![0x22u8; 100])
    );
    drop(second);
    assert!(!refused(), "dropping the successor releases the lock");
    cleanup(&root);
}
