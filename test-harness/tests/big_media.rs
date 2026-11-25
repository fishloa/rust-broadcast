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
    let _serial = PEAK_LOCK.lock().unwrap();
    let (peak, made) = A.peak_during(|| {
        test_harness::big_media::write_big_ts(&out, &test_fixtures::path("ts/h264-aac.ts"), TARGET)
    });
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
    let _serial = PEAK_LOCK.lock().unwrap();
    let (peak, made) =
        A.peak_during(|| test_harness::big_media::write_big_flv(&out, &source, TARGET));
    assert!(made.bytes >= TARGET);
    assert_eq!(std::fs::metadata(&out).unwrap().len(), made.bytes);
    assert_eq!(made.samples_per_track.len(), 2);
    assert!(peak < WRITER_BUDGET, "writer held {peak}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn written_big_files_demux_back_to_every_sample() {
    let _serial = PEAK_LOCK.lock().unwrap();
    let dir = temp_dir("demux");
    let ts_path = dir.join("round-trip.ts");
    let ts = test_harness::big_media::write_big_ts(
        &ts_path,
        &test_fixtures::path("ts/h264-aac.ts"),
        4 << 20,
    );
    let ts_bytes = std::fs::read(&ts_path).unwrap();
    let ts_media = test_harness::collect::media_from_ts(ts_bytes.as_slice()).unwrap();
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
    let flv_media = test_harness::collect::media_from_flv(flv_bytes.as_slice()).unwrap();
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

const DRIFT_TARGET: u64 = 32 << 20;

fn av_pair(media: &transmux::Media) -> (&transmux::Track, &transmux::Track) {
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
        .expect("video track");
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Aac { .. }))
        .expect("audio track");
    (video, audio)
}

fn pts_seconds(track: &transmux::Track, index: usize) -> f64 {
    let pts = track.samples[index]
        .timing
        .as_ref()
        .and_then(|t| t.pts())
        .expect("sample pts");
    pts as f64 / f64::from(track.spec.timescale.get())
}

fn assert_no_drift(
    written: &transmux::Media,
    source: &transmux::Media,
    made: &test_harness::big_media::BigMedia,
    label: &str,
) {
    let (video, audio) = av_pair(written);
    let (source_video, source_audio) = av_pair(source);
    let video_per_pass = source_video.samples.len();
    let audio_per_pass = source_audio.samples.len();
    let passes = video.samples.len() / video_per_pass;
    assert_eq!(video.samples.len() % video_per_pass, 0);
    assert_eq!(audio.samples.len(), passes * audio_per_pass);
    assert!(passes >= 200, "{label}: only {passes} passes");
    assert_eq!(
        made.samples_per_track.len(),
        written.tracks.len(),
        "{label}: every track counted"
    );
    let base = pts_seconds(video, 0) - pts_seconds(audio, 0);
    let last = pts_seconds(video, (passes - 1) * video_per_pass)
        - pts_seconds(audio, (passes - 1) * audio_per_pass);
    let coarser = video.spec.timescale.get().min(audio.spec.timescale.get());
    let tolerance = 1.0 / f64::from(coarser);
    assert!(
        (last - base).abs() <= tolerance,
        "{label}: A/V skew drifted from {base} s to {last} s over {passes} passes"
    );
}

#[test]
fn tracks_do_not_drift_apart_across_passes() {
    let _serial = PEAK_LOCK.lock().unwrap();
    let dir = temp_dir("drift");
    let ts_source = test_fixtures::path("ts/h264-aac.ts");
    let ts_out = dir.join("drift.ts");
    let ts_made = test_harness::big_media::write_big_ts(&ts_out, &ts_source, DRIFT_TARGET);
    let ts_source_bytes = std::fs::read(&ts_source).unwrap();
    let ts_source_media = test_harness::collect::media_from_ts(ts_source_bytes.as_slice()).unwrap();
    let ts_bytes = std::fs::read(&ts_out).unwrap();
    let ts_written = test_harness::collect::media_from_ts(ts_bytes.as_slice()).unwrap();
    assert_no_drift(&ts_written, &ts_source_media, &ts_made, "ts");

    let flv_source = test_fixtures::find("flv/av.flv").expect("flv fixture").0;
    let flv_out = dir.join("drift.flv");
    let flv_made = test_harness::big_media::write_big_flv(&flv_out, &flv_source, DRIFT_TARGET);
    let flv_source_bytes = std::fs::read(&flv_source).unwrap();
    let flv_source_media =
        test_harness::collect::media_from_flv(flv_source_bytes.as_slice()).unwrap();
    let flv_bytes = std::fs::read(&flv_out).unwrap();
    let flv_written = test_harness::collect::media_from_flv(flv_bytes.as_slice()).unwrap();
    assert_no_drift(&flv_written, &flv_source_media, &flv_made, "flv");

    let _ = std::fs::remove_dir_all(&dir);
}
