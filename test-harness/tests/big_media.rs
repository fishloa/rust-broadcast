#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

const TARGET: u64 = 256 << 20;
const WRITER_BUDGET: usize = 32 << 20;

static PEAK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "test-harness-big-media-{}-{tag}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn big_ts_is_written_incrementally_and_reaches_its_size() {
    let dir = temp_dir("ts");
    let out = dir.join("big.ts");
    let (peak, made) = {
        let _serial = PEAK_LOCK.lock().unwrap();
        A.peak_during(|| {
            test_harness::big_media::write_big_ts(
                &out,
                &test_fixtures::path("ts/h264-aac.ts"),
                TARGET,
            )
        })
    };
    assert!(made.bytes >= TARGET);
    assert_eq!(std::fs::metadata(&out).unwrap().len(), made.bytes);
    assert_eq!(made.bytes % 188, 0);
    assert!(peak < WRITER_BUDGET, "writer held {peak}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn big_flv_is_written_incrementally_and_reaches_its_size() {
    let dir = temp_dir("flv");
    let out = dir.join("big.flv");
    let source = test_fixtures::find("flv/av.flv").expect("flv fixture").0;
    let (peak, made) = {
        let _serial = PEAK_LOCK.lock().unwrap();
        A.peak_during(|| test_harness::big_media::write_big_flv(&out, &source, TARGET))
    };
    assert!(made.bytes >= TARGET);
    assert_eq!(std::fs::metadata(&out).unwrap().len(), made.bytes);
    assert_eq!(made.samples_per_track.len(), 2);
    assert!(peak < WRITER_BUDGET, "writer held {peak}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn written_big_files_demux_back_to_every_sample() {
    use broadcast_common::Unpackage;

    let dir = temp_dir("demux");
    let ts_path = dir.join("round-trip.ts");
    let ts = test_harness::big_media::write_big_ts(
        &ts_path,
        &test_fixtures::path("ts/h264-aac.ts"),
        4 << 20,
    );
    let ts_bytes = std::fs::read(&ts_path).unwrap();
    let ts_media = transmux::TsDemux::new()
        .unpackage(ts_bytes.as_slice())
        .unwrap();
    let ts_counts: Vec<(u32, u64)> = ts_media
        .tracks
        .iter()
        .map(|t| {
            (
                t.spec.track_id.value(),
                u64::try_from(t.samples.len()).unwrap(),
            )
        })
        .collect();
    assert_eq!(ts_counts, ts.samples_per_track);
    let video = ts_media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
        .expect("video track survives");
    let dts: Vec<i64> = video
        .samples
        .iter()
        .filter_map(|s| s.timing.as_ref().and_then(|t| t.dts()))
        .collect();
    assert!(dts.windows(2).all(|w| w[0] <= w[1]), "decode order holds");
    assert_eq!(
        dts.len(),
        video.samples.len(),
        "every sample is timestamped"
    );

    let flv_path = dir.join("round-trip.flv");
    let flv = test_harness::big_media::write_big_flv(
        &flv_path,
        &test_fixtures::find("flv/av.flv").expect("flv fixture").0,
        2 << 20,
    );
    let flv_bytes = std::fs::read(&flv_path).unwrap();
    let flv_media = transmux::FlvDemux::new()
        .unpackage(flv_bytes.as_slice())
        .unwrap();
    let flv_counts: Vec<(u32, u64)> = flv_media
        .tracks
        .iter()
        .map(|t| {
            (
                t.spec.track_id.value(),
                u64::try_from(t.samples.len()).unwrap(),
            )
        })
        .collect();
    assert_eq!(flv_counts, flv.samples_per_track);
    assert!(flv.duration > std::time::Duration::ZERO);
    let _ = std::fs::remove_dir_all(&dir);
}
