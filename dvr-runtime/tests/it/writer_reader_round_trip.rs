//! The `fs` recorder writes real files; the `fs` archive reads them back
//! byte-exactly (DESIGN §3.7).

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::MediaPlaylist;
use bytes::Bytes;
use dvr_runtime::fs::{FsArchive, FsRecorder};
use dvr_runtime::read::parse_period_playlist;
use dvr_runtime::writer::InitGeneration;
use dvr_runtime::{ArchiveConfig, ArchiveContainer, ArchiveName, RetentionLimits};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk, TrunkConfig};

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

fn publish(writer: &SegmentWriter, seq: u32, len: usize, fill: u8, at: u64, disc: bool) {
    let entry = SegmentEntry::new(
        Bytes::from(vec![fill; len]),
        SegmentSeq::new(seq),
        Duration::from_secs(2),
        Timestamp::from_nanos(at),
        transmux::SegmentMeta {
            discontinuous: disc,
        },
    );
    writer.publish_segment(entry).expect("publish");
}

fn set_readonly(dir: &std::path::Path, readonly: bool) {
    let mut permissions = std::fs::metadata(dir).expect("meta").permissions();
    permissions.set_readonly(readonly);
    std::fs::set_permissions(dir, permissions).expect("permissions");
}

fn recorder(
    root: &std::path::Path,
    name: &str,
    config: ArchiveConfig,
    trunk: &Arc<Trunk>,
) -> FsRecorder {
    FsRecorder::new(root, ArchiveName::new(name).expect("name"), config, trunk).expect("recorder")
}

#[test]
fn fs_recorder_output_reads_back_exactly() {
    let root = temp_dir("round-trip-fmp4");
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let mut config = config(ArchiveContainer::Fmp4, 2, periods(10));
    config.period_duration = Some(Duration::from_secs(1));
    let mut recorder = recorder(&root, "cam", config, &trunk);
    let init = Bytes::from(vec![0x49u8; 812]);
    let now = Timestamp::ZERO;
    recorder
        .poll(
            now,
            |_| None,
            &[InitGeneration::new(SegmentSeq::new(0), init.clone())],
        )
        .expect("poll init");
    publish(&writer, 0, 100, 0xA0, 0, false);
    recorder.poll(now, |_| None, &[]).expect("p0 segment 0");
    publish(&writer, 1, 200, 0xA1, 2_000_000_000, true);
    recorder
        .poll(Timestamp::from_nanos(2_000_000_000), |_| None, &[])
        .expect("p0 segment 1");
    publish(&writer, 2, 300, 0xA2, 4_000_000_000, false);
    recorder
        .poll(Timestamp::from_nanos(4_000_000_000), |_| None, &[])
        .expect("p1 segment 2");
    recorder
        .poll(Timestamp::from_nanos(6_000_000_000), |_| None, &[])
        .expect("close p1");

    let mut archive = FsArchive::open(
        &root,
        &ArchiveName::new("cam").expect("name"),
        ArchiveContainer::Fmp4,
    );
    let scan = archive.scan().expect("scan");
    assert!(scan.unreadable.is_empty(), "{:?}", scan.unreadable);
    assert_eq!(scan.segments.len(), 3);
    let expected: [(u32, u32, u64, u64, u8, bool); 3] = [
        (0, 0, 812, 100, 0xA0, false),
        (1, 1, 812, 200, 0xA1, true),
        (2, 2, 812, 300, 0xA2, false),
    ];
    for (segment, (seq, period, offset, len, fill, disc)) in scan.segments.iter().zip(expected) {
        assert_eq!(segment.seq, SegmentSeq::new(seq));
        assert_eq!(segment.period.value(), period);
        assert_eq!(segment.span.offset, offset);
        assert_eq!(segment.span.len.get(), len);
        assert_eq!(segment.discontinuous, disc);
        let init_span = segment.init.expect("fmp4 init");
        assert_eq!(init_span.offset, 0);
        assert_eq!(init_span.len.get(), 812);
        assert_eq!(
            archive.read_init(segment.period, init_span).expect("init"),
            init
        );
        let bytes = archive.read(segment).expect("read");
        assert_eq!(bytes.len(), len as usize);
        assert!(
            bytes.iter().all(|&b| b == fill),
            "seq {seq} must be byte-exact"
        );
    }
    for period in [
        dvr_runtime::PeriodNum::new(0),
        dvr_runtime::PeriodNum::new(1),
        dvr_runtime::PeriodNum::new(2),
    ] {
        let path = root.join("cam").join(format!("p{period}.m3u8"));
        let text = std::fs::read_to_string(path).expect("playlist");
        MediaPlaylist::parse(&text).expect("a standard playlist parses");
        parse_period_playlist(period, ArchiveContainer::Fmp4, text.as_bytes())
            .expect("archive parse");
    }
    let p0 = std::fs::read_to_string(root.join("cam").join("p0.m3u8")).expect("p0");
    assert!(p0.ends_with("#EXT-X-ENDLIST\n"), "{p0}");
    assert_eq!(p0.matches("#EXT-X-MAP:").count(), 1);
    cleanup(&root);
}

#[test]
fn fs_recorder_ts_periods_read_back_exactly() {
    let root = temp_dir("round-trip-ts");
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let mut config = config(ArchiveContainer::Ts, 2, periods(10));
    config.period_duration = Some(Duration::from_secs(1));
    let mut recorder = recorder(&root, "ts", config, &trunk);
    publish(&writer, 0, 188, 0x47, 0, false);
    recorder
        .poll(Timestamp::ZERO, |_| None, &[])
        .expect("segment 0");
    publish(&writer, 1, 376, 0x47, 2_000_000_000, true);
    recorder
        .poll(Timestamp::from_nanos(2_000_000_000), |_| None, &[])
        .expect("segment 1");
    let mut archive = FsArchive::open(
        &root,
        &ArchiveName::new("ts").expect("name"),
        ArchiveContainer::Ts,
    );
    let scan = archive.scan().expect("scan");
    assert_eq!(scan.segments.len(), 2);
    assert!(scan.segments.iter().all(|s| s.init.is_none()));
    assert_eq!(archive.read(&scan.segments[0]).expect("read").len(), 188);
    assert_eq!(archive.read(&scan.segments[1]).expect("read").len(), 376);
    assert_eq!(scan.segments[0].period.value(), 0);
    assert_eq!(scan.segments[1].period.value(), 1);
    assert_eq!(scan.segments[1].span.offset, 0);
    cleanup(&root);
}

#[test]
fn a_stale_offset_resyncs_and_the_segment_retries() {
    let root = temp_dir("stale");
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let mut config = config(ArchiveContainer::Ts, 2, periods(10));
    config.period_duration = Some(Duration::from_secs(86400));
    let mut recorder = recorder(&root, "stale", config, &trunk);
    publish(&writer, 0, 100, 0x11, 0, false);
    recorder
        .poll(Timestamp::ZERO, |_| None, &[])
        .expect("segment 0");

    let media = root.join("stale").join("p0.ts");
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&media)
            .expect("open");
        file.write_all(b"JUNK5").expect("junk");
    }
    publish(&writer, 1, 100, 0x22, 2_000_000_000, false);
    recorder
        .poll(Timestamp::from_nanos(2_000_000_000), |_| None, &[])
        .expect("mismatch retried");
    recorder
        .poll(Timestamp::from_nanos(2_000_000_000), |_| None, &[])
        .expect("retry lands");

    assert_eq!(std::fs::metadata(&media).expect("meta").len(), 200);
    let mut archive = FsArchive::open(
        &root,
        &ArchiveName::new("stale").expect("name"),
        ArchiveContainer::Ts,
    );
    let scan = archive.scan().expect("scan");
    assert_eq!(scan.segments.len(), 2);
    let bytes = archive.read(&scan.segments[1]).expect("read");
    assert_eq!(bytes, Bytes::from(vec![0x22u8; 100]));
    cleanup(&root);
}

#[test]
fn a_partial_playlist_append_truncates_and_retries() {
    let root = temp_dir("playlist-retry");
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let mut config = config(ArchiveContainer::Ts, 2, periods(10));
    config.period_duration = Some(Duration::from_secs(86400));
    let mut recorder = recorder(&root, "pl", config, &trunk);
    publish(&writer, 0, 100, 0x11, 0, false);
    recorder
        .poll(Timestamp::ZERO, |_| None, &[])
        .expect("segment 0");
    let playlist = root.join("pl").join("p0.m3u8");
    let created = std::fs::metadata(&playlist).expect("meta").len();
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&playlist)
            .expect("open");
        file.write_all(b"JUNK").expect("junk");
    }
    publish(&writer, 1, 100, 0x22, 2_000_000_000, false);
    recorder
        .poll(Timestamp::from_nanos(2_000_000_000), |_| None, &[])
        .expect("mismatch retried");
    assert_eq!(std::fs::metadata(&playlist).expect("meta").len(), created);
    recorder
        .poll(Timestamp::from_nanos(2_000_000_000), |_| None, &[])
        .expect("retry lands");
    let mut archive = FsArchive::open(
        &root,
        &ArchiveName::new("pl").expect("name"),
        ArchiveContainer::Ts,
    );
    let scan = archive.scan().expect("scan");
    assert_eq!(scan.segments.len(), 2);
    assert_eq!(scan.segments[1].seq, SegmentSeq::new(1));
    cleanup(&root);
}

#[test]
fn a_failed_roll_leaves_the_next_period_starting_with_the_init() {
    let root = temp_dir("failed-roll");
    let trunk = trunk();
    let writer = trunk.segment_writer().expect("writer");
    let mut config = config(ArchiveContainer::Fmp4, 2, periods(10));
    config.period_duration = Some(Duration::from_secs(1));
    let mut recorder = recorder(&root, "r", config, &trunk);
    let init = Bytes::from(vec![0x49u8; 812]);
    recorder
        .poll(
            Timestamp::ZERO,
            |_| None,
            &[InitGeneration::new(SegmentSeq::new(0), init.clone())],
        )
        .expect("init");
    publish(&writer, 0, 100, 0x11, 0, false);
    recorder.poll(Timestamp::ZERO, |_| None, &[]).expect("p0");

    set_readonly(&root.join("r"), true);
    publish(&writer, 1, 100, 0x22, 2_000_000_000, false);
    let error = recorder
        .poll(Timestamp::from_nanos(2_000_000_000), |_| None, &[])
        .expect_err("the roll must fail");
    set_readonly(&root.join("r"), false);
    assert!(matches!(
        error,
        dvr_runtime::fs::FsError::Io {
            op: dvr_runtime::writer::StoreOpKind::CreateData,
            ..
        }
    ));

    publish(&writer, 2, 100, 0x33, 4_000_000_000, false);
    recorder
        .poll(Timestamp::from_nanos(4_000_000_000), |_| None, &[])
        .expect("the retried roll works");
    let p1 = std::fs::read(root.join("r").join("p1.m4s")).expect("p1");
    assert!(p1.starts_with(&init), "p1 must start with the init");
    let mut tail = vec![0x22u8; 100];
    tail.extend_from_slice(&[0x33u8; 100]);
    assert_eq!(
        &p1[init.len()..],
        tail.as_slice(),
        "the failed item is retried before the next one"
    );
    assert!(
        !root.join("r").join("p2.m4s").exists(),
        "the failed roll consumed no period"
    );
    cleanup(&root);
}

#[test]
fn retention_removes_the_files_from_disk() {
    let root = temp_dir("retention-files");
    let name = ArchiveName::new("ret").expect("name");
    let retention = RetentionLimits::new().with_periods(nz(1));
    let base = config(ArchiveContainer::Ts, 2, retention)
        .with_period_duration(Some(Duration::from_secs(1)));
    for round in 0..4u32 {
        let trunk = self::trunk();
        let writer = trunk.segment_writer().expect("writer");
        let mut recorder = recorder(
            &root,
            "ret",
            base.clone().with_seq_offset(u64::from(round)),
            &trunk,
        );
        publish(&writer, 0, 100, 0x10 + round as u8, 0, false);
        recorder
            .poll(Timestamp::ZERO, |_| None, &[])
            .expect("segment");
        recorder
            .poll(Timestamp::from_nanos(2_000_000_000), |_| None, &[])
            .expect("close");
    }
    for n in [0u32, 1] {
        assert!(
            !root.join("ret").join(format!("p{n}.m3u8")).exists(),
            "p{n} playlist"
        );
        assert!(
            !root.join("ret").join(format!("p{n}.ts")).exists(),
            "p{n} media"
        );
    }
    assert!(root.join("ret").join("p2.ts").exists());
    assert!(root.join("ret").join("p3.ts").exists());
    let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Ts);
    assert_eq!(archive.cached_periods(), 0);
    let scan = archive.scan().expect("scan");
    assert_eq!(scan.segments.len(), 2);
    assert_eq!(scan.segments[0].period.value(), 2);
    assert_eq!(scan.segments[1].period.value(), 3);
    assert_eq!(scan.segments[0].seq, SegmentSeq::new(2));
    assert_eq!(scan.segments[1].seq, SegmentSeq::new(3));
    assert!(scan.conflicts.is_empty());
    cleanup(&root);
}

#[test]
fn a_persistently_failing_item_becomes_a_counted_gap() {
    use dvr_runtime::writer::WriterEvent;
    let root = temp_dir("retry-budget");
    let trunk = trunk();
    let mut config = config(ArchiveContainer::Ts, 2, periods(10));
    config.period_duration = Some(Duration::from_secs(86400));
    let mut recorder = recorder(&root, "gap", config, &trunk);
    std::fs::create_dir_all(root.join("gap")).expect("archive directory");
    set_readonly(&root.join("gap"), true);
    publish(
        &trunk.segment_writer().expect("writer"),
        0,
        100,
        0x47,
        0,
        false,
    );
    for _ in 0..4 {
        let error = recorder
            .poll(Timestamp::ZERO, |_| None, &[])
            .expect_err("the blocked create fails");
        assert!(matches!(error, dvr_runtime::fs::FsError::Io { .. }));
    }
    set_readonly(&root.join("gap"), false);
    assert!(!root.join("gap").join("p0.ts").exists());
    assert_eq!(recorder.writer().counters().gaps, 1);
    let events: Vec<WriterEvent> = std::iter::from_fn(|| recorder.poll_event()).collect();
    assert!(
        events.contains(&WriterEvent::Gap { skipped: 1 }),
        "{events:?}"
    );
    recorder
        .poll(Timestamp::ZERO, |_| None, &[])
        .expect("nothing left");
    cleanup(&root);
}
