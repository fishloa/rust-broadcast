use super::*;
use media_plane::trunk::{SegmentEntry, Trunk, TrunkConfig};
use std::num::NonZeroUsize;
use std::time::Duration;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn cfg(root: &std::path::Path, retention: usize) -> DvrConfig {
    DvrConfig {
        enabled: true,
        archive_root: root.to_string_lossy().to_string(),
        retention_periods: retention,
        retention_bytes: 0,
        period_duration_secs: 3600,
        overrun: ArchiveOverrunSerde::Gap,
        dvb_service_id: None,
    }
}

fn entry(seq: u32) -> SegmentEntry {
    SegmentEntry::new(
        bytes::Bytes::from(vec![seq as u8; 16]),
        SegmentSeq::new(seq),
        Duration::from_secs(2),
        broadcast_common::Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
        transmux::SegmentMeta {
            discontinuous: false,
        },
    )
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("auditf-dvr-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn a_period_opened_after_a_failed_roll_still_starts_with_the_init() {
    let root = temp_dir("failedroll");
    let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(8), nz(4), nz(4)));
    let writer = trunk.segment_writer().expect("writer");
    let mut rec = DvrRecorder::new("r".into(), cfg(&root, 10), ".m4s", &trunk).expect("rec");
    rec.poll_and_persist(Some(b"INIT")).expect("open p0");
    writer.publish_segment(entry(1)).expect("publish");
    rec.poll_and_persist(Some(b"INIT")).expect("persist 1");

    // The next period cannot be opened (a transient I/O failure; here the
    // path is occupied by a directory).
    let dir = root.join("r");
    fs::create_dir(dir.join("p1.m4s")).expect("blocker");
    rec.period_opened_at = Some(SystemTime::now() - Duration::from_secs(7200));
    assert!(rec.poll_and_persist(Some(b"INIT")).is_err(), "roll must fail");
    fs::remove_dir(dir.join("p1.m4s")).expect("unblock");

    writer.publish_segment(entry(2)).expect("publish");
    rec.poll_and_persist(Some(b"INIT")).expect("persist 2");

    let holder = (1..=3)
        .map(|n| dir.join(format!("p{n}.m4s")))
        .find(|p| p.is_file() && fs::read(p).unwrap().contains(&2u8))
        .expect("some period holds segment 2");
    let bytes = fs::read(&holder).unwrap();
    assert!(
        bytes.starts_with(b"INIT"),
        "{} holds segment 2 but starts with {:?}, not the init",
        holder.display(),
        &bytes[..bytes.len().min(8)]
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn eviction_removes_the_programme_sidecar() {
    let root = temp_dir("eventjson");
    let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(8), nz(4), nz(4)));
    let writer = trunk.segment_writer().expect("writer");
    let mut rec = DvrRecorder::new("r".into(), cfg(&root, 1), ".m4s", &trunk).expect("rec");
    rec.current_programme = Some(EitProgramme {
        event_id: 7,
        service_id: 1,
        title: Some("t".into()),
        start: None,
        duration_secs: None,
    });
    rec.poll_and_persist(Some(b"INIT_0")).expect("p0");
    for (n, init) in [(1u32, b"INIT_1"), (2, b"INIT_2"), (3, b"INIT_3")] {
        writer.publish_segment(entry(n)).expect("publish");
        rec.poll_and_persist(Some(init)).expect("roll");
    }
    let dir = root.join("r");
    let mut leftover: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".event.json"))
        .collect();
    leftover.sort();
    let kept: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".idx"))
        .collect();
    assert!(
        leftover.len() <= kept.len(),
        "retention keeps {kept:?} but .event.json sidecars remain for {leftover:?}"
    );
    let _ = fs::remove_dir_all(&root);
}
