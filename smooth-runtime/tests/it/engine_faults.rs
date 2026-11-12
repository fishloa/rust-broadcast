use smooth_runtime::{Dropped, ManifestFault, Output, SmoothConfig, SmoothError};

use crate::common::{Host, Reply, fixture, is_ended, is_fatal};

const FIRST_VIDEO: &str = "QualityLevels(64528)/Fragments(video=0)";

fn av_host(config: SmoothConfig) -> Host {
    let mut host = Host::new(config);
    host.serve_dir("av");
    host
}

#[test]
fn a_transient_fragment_failure_is_retried_and_playback_completes() {
    let good = fixture(&format!("av/{FIRST_VIDEO}"));
    let mut host = av_host(SmoothConfig::default());
    host.route(
        FIRST_VIDEO,
        vec![Reply::Transport, Reply::Status(503), Reply::Body(good)],
    );
    host.run_until(|o| o.iter().any(is_ended));
    assert_eq!(host.count_requests(FIRST_VIDEO), 3);
    assert!(host.has(is_ended));
}

#[test]
fn retries_wait_out_the_configured_backoff() {
    let good = fixture(&format!("av/{FIRST_VIDEO}"));
    let mut host = av_host(SmoothConfig {
        backoff_ms: vec![1_000],
        ..SmoothConfig::default()
    });
    host.route(FIRST_VIDEO, vec![Reply::Transport, Reply::Body(good)]);
    host.run_until(|o| o.iter().any(is_ended));
    let times = host.times_of(FIRST_VIDEO);
    assert_eq!(times.len(), 2);
    assert_eq!(times[1] - times[0], 1_000_000);
}

#[test]
fn a_fragment_retry_waits_out_the_server_retry_after_over_the_backoff() {
    let good = fixture(&format!("av/{FIRST_VIDEO}"));
    let mut host = av_host(SmoothConfig {
        backoff_ms: vec![1_000],
        ..SmoothConfig::default()
    });
    host.route(
        FIRST_VIDEO,
        vec![Reply::Throttled(503, 2_000_000), Reply::Body(good)],
    );
    host.run_until(|o| o.iter().any(is_ended));
    let times = host.times_of(FIRST_VIDEO);
    assert_eq!(times.len(), 2);
    assert_eq!(times[1] - times[0], 2_000_000);
}

#[test]
fn a_manifest_retry_waits_out_the_server_retry_after_over_the_backoff() {
    let mut host = av_host(SmoothConfig {
        backoff_ms: vec![1_000],
        ..SmoothConfig::default()
    });
    host.route(
        "Manifest",
        vec![
            Reply::Throttled(429, 3_000_000),
            Reply::Body(fixture("av/Manifest")),
        ],
    );
    host.run_until(|o| o.iter().any(is_ended));
    let times = host.times_of("Manifest");
    assert_eq!(times.len(), 2);
    assert_eq!(times[1] - times[0], 3_000_000);
}

#[test]
fn a_static_fragment_that_never_succeeds_is_fatal_after_the_configured_count() {
    let mut host = av_host(SmoothConfig {
        max_fragment_failures: 3,
        ..SmoothConfig::default()
    });
    host.route(FIRST_VIDEO, vec![Reply::Status(500)]);
    host.run_until(|o| o.iter().any(is_fatal));
    assert_eq!(host.count_requests(FIRST_VIDEO), 3);
    assert!(host.has(|o| matches!(
        o,
        Output::Fatal(SmoothError::Fragment { status: Some(500) })
    )));
}

#[test]
fn a_first_manifest_404_reports_not_found_and_retries() {
    let mut host = av_host(SmoothConfig::default());
    host.route(
        "Manifest",
        vec![Reply::Status(404), Reply::Body(fixture("av/Manifest"))],
    );
    host.run_until(|o| o.iter().any(is_ended));
    assert!(host.has(|o| matches!(o, Output::ManifestNotFound)));
    assert!(host.has(|o| matches!(o, Output::Dropped(Dropped::Manifest(ManifestFault::Fetch)))));
    assert!(host.has(is_ended));
}

#[test]
fn a_manifest_that_never_parses_is_fatal_after_the_configured_count() {
    let mut host = Host::new(SmoothConfig {
        max_manifest_failures: 2,
        ..SmoothConfig::default()
    });
    host.route("Manifest", vec![Reply::Body(b"not xml".to_vec())]);
    host.run();
    assert_eq!(host.count_requests("Manifest"), 2);
    assert!(host.has(|o| matches!(
        o,
        Output::Fatal(SmoothError::Manifest(ManifestFault::Parse))
    )));
}

#[test]
fn a_manifest_that_is_not_utf8_is_a_typed_fault() {
    let mut host = Host::new(SmoothConfig {
        max_manifest_failures: 1,
        ..SmoothConfig::default()
    });
    host.route("Manifest", vec![Reply::Body(vec![0xff, 0xfe, 0xfd])]);
    host.run();
    assert!(host.has(|o| matches!(o, Output::Fatal(SmoothError::Manifest(ManifestFault::Utf8)))));
}

#[test]
fn a_manifest_without_a_video_or_audio_stream_is_fatal() {
    let mut host = Host::new(SmoothConfig::default());
    host.route(
        "Manifest",
        vec![Reply::Body(
            b"<SmoothStreamingMedia MajorVersion=\"2\" MinorVersion=\"0\" Duration=\"0\"></SmoothStreamingMedia>"
                .to_vec(),
        )],
    );
    host.run();
    assert!(host.has(|o| matches!(o, Output::Fatal(SmoothError::NoUsableStream))));
}

#[test]
fn a_truncated_fragment_is_a_retried_failure_not_silent_loss() {
    let good = fixture(&format!("av/{FIRST_VIDEO}"));
    let mut host = av_host(SmoothConfig::default());
    host.route(
        FIRST_VIDEO,
        vec![Reply::Body(good[..20].to_vec()), Reply::Body(good)],
    );
    host.run_until(|o| o.iter().any(is_ended));
    assert_eq!(host.count_requests(FIRST_VIDEO), 2);
    assert!(host.has(is_ended));
}

#[test]
fn a_selector_picks_the_requested_video_quality() {
    use smooth_runtime::QualitySelector;
    let text = String::from_utf8(fixture("av/Manifest")).expect("utf8").replace(
        "</StreamIndex>\n<StreamIndex Type=\"audio\"",
        "<QualityLevel Index=\"1\" Bitrate=\"10000\" FourCC=\"H264\" MaxWidth=\"160\" MaxHeight=\"120\" CodecPrivateData=\"00000001674d400bd902847e5c0440000003004000000c83c50a920000000168ebccb2\" />\n</StreamIndex>\n<StreamIndex Type=\"audio\"",
    );
    let mut host = Host::new(SmoothConfig {
        video_quality: QualitySelector::new(|q| {
            q.iter()
                .enumerate()
                .min_by_key(|(_, i)| i.bitrate)
                .map_or(0, |(n, _)| n)
        }),
        ..SmoothConfig::default()
    });
    host.serve_dir("av");
    host.route("Manifest", vec![Reply::Body(text.into_bytes())]);
    host.run_until(|o| o.iter().any(|o| matches!(o, Output::Tracks(_))));
    assert!(
        host.requested
            .iter()
            .any(|u| u.contains("QualityLevels(10000)/Fragments(video=0)"))
    );
}

const TFXD_TIME_RANGE: std::ops::Range<usize> = 304..312;

#[test]
fn a_tfxd_time_beyond_i64_drops_the_fragment_without_panicking() {
    let mut hostile = fixture(&format!("av/{FIRST_VIDEO}"));
    hostile[TFXD_TIME_RANGE].fill(0xFF);
    let mut host = av_host(SmoothConfig::default());
    host.route(FIRST_VIDEO, vec![Reply::Body(hostile)]);
    host.run_until(|o| o.iter().any(is_ended));
    assert!(host.has(|o| matches!(o, Output::Dropped(Dropped::Fragment { status: None, .. }))));
    assert!(host.has(is_ended));
}

fn av_manifest_with_audio_chunks(chunks: &str) -> Vec<u8> {
    String::from_utf8(fixture("av/Manifest"))
        .expect("utf8")
        .replace(
            "<c n=\"0\" d=\"10214580\" />\n<c n=\"1\" d=\"9984581\" />\n<c n=\"2\" d=\"10216780\" />",
            chunks,
        )
        .into_bytes()
}

#[test]
fn a_stream_whose_timeline_cannot_be_enumerated_ends_the_session_with_a_typed_cause() {
    let mut host = av_host(SmoothConfig::default());
    host.route(
        "Manifest",
        vec![Reply::Body(av_manifest_with_audio_chunks("<c n=\"0\" />"))],
    );
    host.run_until(|o| o.iter().any(|o| is_fatal(o) || is_ended(o)));
    assert!(host.has(|o| matches!(o, Output::Fatal(SmoothError::Timeline { .. }))));
}

#[test]
fn a_zero_timescale_is_never_delivered() {
    let zero = String::from_utf8(fixture("av/Manifest"))
        .expect("utf8")
        .replace(
            "<SmoothStreamingMedia ",
            "<SmoothStreamingMedia TimeScale=\"0\" ",
        );
    let mut host = av_host(SmoothConfig::default());
    host.route("Manifest", vec![Reply::Body(zero.into_bytes())]);
    host.run_until(|o| o.iter().any(|o| is_fatal(o) || is_ended(o)));
    assert!(!host.has(|o| matches!(o, Output::Tracks(_))));
    assert!(host.has(|o| matches!(o, Output::Fatal(SmoothError::NoUsableStream))));
}
