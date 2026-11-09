use std::path::PathBuf;
use std::sync::Arc;

use media_plane::ingress::ProgramId;

use crate::dvr::{ArchiveOverrunSerde, DvrConfig};
use crate::route::RouteHandle;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("auditf-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn dvr(root: &std::path::Path) -> DvrConfig {
    DvrConfig {
        enabled: true,
        archive_root: root.to_string_lossy().to_string(),
        retention_periods: 10,
        retention_bytes: 0,
        period_duration_secs: 3600,
        overrun: ArchiveOverrunSerde::Gap,
        dvb_service_id: None,
    }
}

fn seg(seq: u32, byte: u8) -> transmux::ll_hls::SegmentInfo {
    transmux::ll_hls::SegmentInfo {
        bytes: vec![byte; 24],
        duration: 2.0,
        segment_seq: seq,
        part_count: 0,
    }
}

fn archived(dir: &std::path::Path) -> Vec<(u32, u32)> {
    crate::catchup::scan_archive(dir)
        .iter()
        .map(|s| (s.seq.value(), s.period_num))
        .collect()
}

#[tokio::test]
async fn segments_closed_under_the_old_init_are_archived_with_the_old_init() {
    let root = temp_dir("initchange");
    let route = Arc::new(RouteHandle::new(2.0, 500, 8).with_name("r").with_dvr(dvr(&root)));
    let p = ProgramId(0);
    route.publish_new_program(p);
    route.set_init(p, b"INIT_A".to_vec());
    route.add_segment(p, seg(1, 0xA1)).expect("add");
    route.drain_dvr().await;

    // The segmenter closes the last segment of the old track set and installs
    // the new init in the same pass, before the DVR drain runs.
    route.add_segment(p, seg(2, 0xA2)).expect("add");
    route.set_init(p, b"INIT_B".to_vec());
    route.add_segment(p, seg(3, 0xB3)).expect("add");
    route.drain_dvr().await;

    let periods = archived(&root.join("r"));
    assert_eq!(
        periods,
        vec![(1, 0), (2, 0), (3, 1)],
        "segment 2 was closed under INIT_A but is archived in the INIT_B period"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn segments_closed_before_a_reconnect_are_archived() {
    let root = temp_dir("reconnect");
    let route = Arc::new(RouteHandle::new(2.0, 500, 8).with_name("r").with_dvr(dvr(&root)));
    let p = ProgramId(0);
    let first = route.publish_new_program(p);
    route.set_init(p, b"INIT_A".to_vec());
    route.add_segment(p, seg(1, 0xA1)).expect("add");
    route.drain_dvr().await;
    route.add_segment(p, seg(2, 0xA2)).expect("add");

    route.release_program(p, &first);
    route.publish_new_program(p);
    route.set_init(p, b"INIT_A".to_vec());
    route.add_segment(p, seg(1, 0xB1)).expect("add");
    route.drain_dvr().await;

    let seqs: Vec<u32> = archived(&root.join("r")).iter().map(|s| s.0).collect();
    assert_eq!(
        seqs,
        vec![1, 2, 4],
        "segment 2 was closed on the old Trunk before the reconnect and never archived"
    );
    let _ = std::fs::remove_dir_all(&root);
}
