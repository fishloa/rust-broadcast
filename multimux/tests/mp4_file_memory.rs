//! MP4 file playout holds memory bounded by the session's read window, never
//! the file's length (streaming-first Phase 3 gate, issue #4057).
//!
//! Three 8 GiB files are generated at test time as sparse images (progressive
//! `moov`-first, progressive `moov`-last and fragmented), played through the
//! real `FileReader` and through a complete file route served as LL-HLS. The
//! process-wide peak allocator is the hard bound; a looping pass must not grow
//! the peak.

#![cfg(target_pointer_width = "64")]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use media_plane::trunk::{SampleCursorItem, Trunk, TrunkConfig};
use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::source::file_reader::{FileReader, FileReaderConfig};
use test_harness::big_media::{BigFmp4Options, BigMp4Options, write_big_fmp4, write_big_mp4};

#[path = "support/http.rs"]
mod http;
#[path = "support/listener.rs"]
mod listener;

#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

/// Generated file size: far above the memory budget.
const FILE_BYTES: u64 = 8 << 30;
/// Live bytes a playout may hold while reading it.
const BUDGET: usize = 64 << 20;
/// Samples per track in the generated progressive files.
const SAMPLE_COUNT: u32 = 20_000;
/// Fragments (one sample each) in the generated fragmented file.
const FRAGMENT_COUNT: u32 = 16_384;
/// Bytes in one generated fragment sample.
const FRAGMENT_SAMPLE_BYTES: u32 = 512 << 10;
/// Timed samples the trunk retains: a few drains' worth.
const RING: usize = 64;
/// How much a second loop pass may raise the peak above the first.
const LOOP_PEAK_SLACK: usize = 1 << 20;

type Digest = (u32, i64, i64, Option<u32>, bool, Vec<u8>);

fn trunk() -> Arc<Trunk> {
    let nz = std::num::NonZeroUsize::new(RING).unwrap_or(std::num::NonZeroUsize::MIN);
    Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz)
            .with_sparse_capacity(std::num::NonZeroUsize::MIN)
            .with_segment_capacity(std::num::NonZeroUsize::MIN)
            .with_event_capacity(std::num::NonZeroUsize::MIN)
            .with_part_capacity(std::num::NonZeroUsize::MIN),
    )
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("multimux-mp4-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Play `path` through the real reader, draining the trunk concurrently, and
/// return the peak bytes above the entry baseline.
async fn play(path: &Path, max_loops: Option<u32>) -> usize {
    let trunk = trunk();
    let mut cursor = trunk.subscribe_from_backlog();
    let config = FileReaderConfig::new(path.to_path_buf(), max_loops.is_some(), trunk.clone())
        .with_pace(false)
        .with_max_retries(0)
        .with_max_loops(max_loops);
    A.reset();
    let baseline = A.live();
    let handle = tokio::spawn(FileReader::new(config).run());
    loop {
        while let Some(item) = cursor.poll() {
            if let SampleCursorItem::Lagged { .. } = item {
                panic!("the drain loop must keep up with the trunk ring");
            }
        }
        if handle.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    handle
        .await
        .expect("the reader task must not panic")
        .expect("the generated MP4 must play cleanly");
    A.peak().saturating_sub(baseline)
}

/// Play `path` once, returning every sample's digest.
async fn collect(path: &Path, max_loops: Option<u32>) -> Vec<Digest> {
    let trunk = trunk();
    let mut cursor = trunk.subscribe_from_backlog();
    let config = FileReaderConfig::new(path.to_path_buf(), max_loops.is_some(), trunk.clone())
        .with_pace(false)
        .with_max_retries(0)
        .with_max_loops(max_loops);
    let handle = tokio::spawn(FileReader::new(config).run());
    let mut out = Vec::new();
    loop {
        while let Some(item) = cursor.poll() {
            if let SampleCursorItem::Timed {
                track_id, sample, ..
            } = item
                && let Some(timing) = sample.timing
            {
                out.push((
                    track_id.value(),
                    timing.pts().unwrap_or(0),
                    timing.dts().unwrap_or(0),
                    timing.duration(),
                    sample.flags.is_sync,
                    sample.data.to_vec(),
                ));
            }
        }
        if handle.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    handle
        .await
        .expect("the reader task must not panic")
        .expect("the looped MP4 must play cleanly");
    out
}

fn one_ll_hls_route(bind: SocketAddr, path: String) -> Config {
    Config::default()
        .with_bind(bind.to_string())
        .with_target_duration(Duration::from_millis(500))
        .with_part_target(Duration::from_millis(100))
        .with_window_segments(8)
        .with_routes(vec![Route {
            name: "cam".to_string(),
            input: InputSpec::File {
                path,
                loop_file: false,
            },
            outputs: vec![OutputKind::LlHls],
            dvr: DvrConfig::default(),
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
        }])
}

/// How long the harness waits for the first closed segment.
const HANG_GUARD: Duration = Duration::from_secs(60);

async fn poll_until_extinf(client: &reqwest::Client, playlist_url: &str) -> String {
    let blocking = format!("{playlist_url}?_HLS_msn=0");
    let deadline = tokio::time::Instant::now() + HANG_GUARD;
    loop {
        let wait = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !wait.is_zero(),
            "no #EXTINF: line appeared in {playlist_url} within the hang guard"
        );
        if let Ok(Ok(resp)) = tokio::time::timeout(wait, client.get(&blocking).send()).await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return body;
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn mp4_file_playout_holds_less_than_the_budget() {
    let dir = scratch();

    let progressive = dir.join("progressive.mp4");
    let made = write_big_mp4(
        &progressive,
        &BigMp4Options {
            target_bytes: FILE_BYTES,
            sample_count: SAMPLE_COUNT,
            moov_first: true,
            ..BigMp4Options::default()
        },
    );
    assert!(made.bytes >= FILE_BYTES, "the generated file is 8 GiB");
    assert!(
        made.samples_per_track
            .iter()
            .all(|(_, n)| *n == u64::from(SAMPLE_COUNT)),
        "each generated track carries every sample"
    );
    let held = play(&progressive, Some(1)).await;
    eprintln!("progressive moov-first: peak {held} bytes above baseline");
    assert!(
        held < BUDGET,
        "an 8 GiB progressive MP4 held {held} live bytes, over the {BUDGET} budget"
    );

    let moov_last = dir.join("moov-last.mp4");
    let made = write_big_mp4(
        &moov_last,
        &BigMp4Options {
            target_bytes: FILE_BYTES,
            sample_count: SAMPLE_COUNT,
            moov_first: false,
            ..BigMp4Options::default()
        },
    );
    assert!(made.bytes >= FILE_BYTES, "the generated file is 8 GiB");
    let held = play(&moov_last, Some(1)).await;
    eprintln!("progressive moov-last: peak {held} bytes above baseline");
    assert!(
        held < BUDGET,
        "an 8 GiB moov-last MP4 held {held} live bytes, over the {BUDGET} budget"
    );

    let fragmented = dir.join("fragmented.mp4");
    let made = write_big_fmp4(
        &fragmented,
        &BigFmp4Options {
            target_bytes: FILE_BYTES,
            fragment_count: FRAGMENT_COUNT,
            sample_bytes: FRAGMENT_SAMPLE_BYTES,
            with_mfra: true,
            protect: false,
        },
    );
    assert!(made.bytes >= FILE_BYTES, "the generated file is 8 GiB");
    let held = play(&fragmented, Some(1)).await;
    eprintln!("fragmented: peak {held} bytes above baseline");
    assert!(
        held < BUDGET,
        "an 8 GiB fragmented MP4 held {held} live bytes, over the {BUDGET} budget"
    );

    // End to end through the serving layer: the same 8 GiB progressive file
    // feeds a real LL-HLS route, and the whole server stays inside the bound.
    let (bind, listener) = listener::bind_tcp();
    let config = one_ll_hls_route(bind, progressive.to_string_lossy().into_owned());
    A.reset();
    let baseline = A.live();
    let server = tokio::spawn(
        multimux::Server::new(
            multimux::ServeRequest::new(config)
                .with_registry(SchemeRegistry::new())
                .with_prebound_media(listener),
        )
        .serve(),
    );
    let client = http::client();
    let playlist_url = format!("http://{bind}/cam/media.m3u8");
    let playlist = poll_until_extinf(&client, &playlist_url).await;
    assert!(
        playlist.contains("#EXTINF:"),
        "the LL-HLS route must serve a closed segment"
    );
    let served = A.peak().saturating_sub(baseline);
    eprintln!("LL-HLS file route: peak {served} bytes above baseline");
    assert!(
        served < BUDGET,
        "serving an 8 GiB MP4 as LL-HLS held {served} live bytes, over the {BUDGET} budget"
    );
    server.abort();

    // Looping: two passes of a small MP4 produce the same per-track sample
    // sequence, and the second pass does not raise the peak.
    let looping = test_fixtures::path("mp4/progressive/av-prog.mp4");
    let single = collect(&looping, Some(1)).await;
    let twice = collect(&looping, Some(2)).await;
    let mut first_per_track: BTreeMap<u32, Vec<&Digest>> = BTreeMap::new();
    for digest in &single {
        first_per_track.entry(digest.0).or_default().push(digest);
    }
    for (track, first) in &first_per_track {
        let of_track: Vec<&Digest> = twice.iter().filter(|d| d.0 == *track).collect();
        assert_eq!(
            of_track.len(),
            first.len() * 2,
            "track {track} must play its whole sample set twice"
        );
        let (pass_one, pass_two) = of_track.split_at(first.len());
        for (expected, actual) in first.iter().zip(pass_one) {
            assert_eq!(**expected, **actual, "track {track} first pass");
        }
        let offset = pass_two[0].2 - pass_one[0].2;
        for (expected, actual) in pass_one.iter().zip(pass_two) {
            assert_eq!(expected.3, actual.3, "track {track} duration");
            assert_eq!(expected.4, actual.4, "track {track} sync flag");
            assert_eq!(expected.5, actual.5, "track {track} sample bytes");
            assert_eq!(actual.2 - expected.2, offset, "track {track} dts step");
        }
    }
    let pass1_peak = play(&looping, Some(1)).await;
    let pass2_peak = play(&looping, Some(2)).await;
    eprintln!("looping pass 1 peak {pass1_peak} bytes, pass 2 peak {pass2_peak} bytes");
    assert!(
        pass2_peak <= pass1_peak.saturating_add(LOOP_PEAK_SLACK),
        "pass 2 peaked at {pass2_peak}, pass 1 at {pass1_peak} (slack {LOOP_PEAK_SLACK})"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
