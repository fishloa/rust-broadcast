use broadcast_common::backoff::{RetrySchedule, mix_seed};
use smooth_runtime::{Dropped, ManifestFault, Output, SmoothClientConfig, SmoothError};

use crate::common::{Host, Reply, fixture, is_ended, is_fatal};

const FIRST_VIDEO: &str = "QualityLevels(64528)/Fragments(video=0)";

fn av_host(config: SmoothClientConfig) -> Host {
    let mut host = Host::new(config);
    host.serve_dir("av");
    host
}

#[test]
fn a_transient_fragment_failure_is_retried_and_playback_completes() {
    let good = fixture(&format!("av/{FIRST_VIDEO}"));
    let mut host = av_host(SmoothClientConfig::default());
    host.route(
        FIRST_VIDEO,
        vec![Reply::Transport, Reply::Status(503), Reply::Body(good)],
    );
    host.run_until(|o| o.iter().any(is_ended));
    assert_eq!(host.count_requests(FIRST_VIDEO), 3);
    assert!(host.has(is_ended));
}

#[test]
fn a_backoff_cap_beyond_the_ceiling_fails_at_construction() {
    let cap = broadcast_common::backoff::MAX_FLOOR + std::time::Duration::from_secs(1);
    let config =
        SmoothClientConfig::default().with_backoff(broadcast_common::backoff::BackoffSteps {
            steps: vec![std::time::Duration::from_millis(1_000)],
            cap,
        });
    let mut host = av_host(config);
    host.run_until(|o| o.iter().any(is_fatal));
    assert!(host.outputs.iter().any(|o| matches!(
        o,
        Output::Fatal(SmoothError::InvalidBackoff(
            broadcast_common::backoff::InvalidBackoff::FloorAboveCeiling { .. }
        ))
    )));
    assert_eq!(host.count_requests(FIRST_VIDEO), 0, "no request goes out");
}

#[test]
fn retries_wait_out_the_configured_backoff() {
    let good = fixture(&format!("av/{FIRST_VIDEO}"));
    let mut host = av_host(SmoothClientConfig::default().with_backoff(steps(&[1_000], 1_000)));
    host.route(FIRST_VIDEO, vec![Reply::Transport, Reply::Body(good)]);
    host.run_until(|o| o.iter().any(is_ended));
    let times = host.times_of(FIRST_VIDEO);
    assert_eq!(times.len(), 2);
    assert_eq!(times[1] - times[0], 1_000_000);
}

const STEPS_MS: [u64; 3] = [2_000, 4_000, 8_000];
const CAP_MS: u64 = 10_000;

/// A step table in milliseconds, then its cap, as the config's one backoff value.
fn steps(steps_ms: &[u64], cap_ms: u64) -> broadcast_common::backoff::BackoffSteps {
    broadcast_common::backoff::BackoffSteps {
        steps: steps_ms
            .iter()
            .map(|ms| std::time::Duration::from_millis(*ms))
            .collect(),
        cap: std::time::Duration::from_millis(cap_ms),
    }
}

/// The waits between the attempts of a fragment that fails `failures` times before it is served.
fn retry_waits_us(seed: u64, failures: usize) -> Vec<u64> {
    let good = fixture(&format!("av/{FIRST_VIDEO}"));
    let mut host = av_host(
        SmoothClientConfig::default()
            .with_backoff(steps(&STEPS_MS, CAP_MS))
            .with_jitter_seed(seed),
    );
    let mut replies = vec![Reply::Transport; failures];
    replies.push(Reply::Body(good));
    host.route(FIRST_VIDEO, replies);
    host.run_until(|o| o.iter().any(is_ended));
    let times = host.times_of(FIRST_VIDEO);
    assert_eq!(times.len(), failures + 1);
    times.windows(2).map(|pair| pair[1] - pair[0]).collect()
}

/// Smooth's retries are the shared `RetrySchedule`: the first failure waits exactly its step, each
/// later one the step times the next jitter term of the seed, and two clients with different seeds
/// do not retry in lockstep.
#[test]
fn retries_follow_the_shared_retry_schedule() {
    let steps: Vec<std::time::Duration> = STEPS_MS
        .iter()
        .map(|ms| std::time::Duration::from_millis(*ms))
        .collect();
    let seed = 7;
    let waits = retry_waits_us(seed, 3);

    let schedule = RetrySchedule::from_steps(
        &steps,
        std::time::Duration::from_millis(CAP_MS),
        mix_seed(seed),
    )
    .unwrap();
    let expected: Vec<u64> = (0..3)
        .map(|attempt| u64::try_from(schedule.delay_for_attempt(attempt).as_micros()).unwrap())
        .collect();
    assert_eq!(
        waits, expected,
        "the waits are the schedule's delays in order"
    );
    assert_eq!(
        waits[0], 2_000_000,
        "the first retry waits exactly its step"
    );
    for (wait, step_ms) in waits.iter().zip(STEPS_MS).skip(1) {
        let step_us = step_ms * 1_000;
        assert!(
            *wait >= step_us * 5 / 6 && *wait <= step_us,
            "{wait} us is outside the jitter band of the {step_ms} ms step"
        );
    }

    let other = retry_waits_us(seed + 1, 3);
    assert_eq!(other[0], waits[0]);
    assert_ne!(other[1..], waits[1..], "different seeds retry apart");
}

#[test]
fn a_fragment_retry_waits_out_the_server_retry_after_over_the_backoff() {
    let good = fixture(&format!("av/{FIRST_VIDEO}"));
    let mut host = av_host(SmoothClientConfig::default().with_backoff(steps(&[1_000], 1_000)));
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
    let mut host = av_host(SmoothClientConfig::default().with_backoff(steps(&[1_000], 1_000)));
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
    let mut host = av_host(SmoothClientConfig::default().with_max_fragment_failures(3));
    host.route(FIRST_VIDEO, vec![Reply::Status(500)]);
    host.run_until(|o| o.iter().any(is_fatal));
    assert_eq!(host.count_requests(FIRST_VIDEO), 3);
    assert!(host.has(|o| matches!(
        o,
        Output::Fatal(SmoothError::Fragment {
            status: Some(http::StatusCode::INTERNAL_SERVER_ERROR),
        })
    )));
}

#[test]
fn a_first_manifest_404_reports_not_found_and_retries() {
    let mut host = av_host(SmoothClientConfig::default());
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
    let mut host = Host::new(SmoothClientConfig::default().with_max_manifest_failures(2));
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
    let mut host = Host::new(SmoothClientConfig::default().with_max_manifest_failures(1));
    host.route("Manifest", vec![Reply::Body(vec![0xff, 0xfe, 0xfd])]);
    host.run();
    assert!(host.has(|o| matches!(o, Output::Fatal(SmoothError::Manifest(ManifestFault::Utf8)))));
}

#[test]
fn a_manifest_without_a_video_or_audio_stream_is_fatal() {
    let mut host = Host::new(SmoothClientConfig::default());
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
    let mut host = av_host(SmoothClientConfig::default());
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
    let mut host = Host::new(SmoothClientConfig::default().with_video_quality(
        QualitySelector::new(|q| {
            q.iter()
                .enumerate()
                .min_by_key(|(_, i)| i.bitrate)
                .map_or(0, |(n, _)| n)
        }),
    ));
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
    let mut host = av_host(SmoothClientConfig::default());
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
    let mut host = av_host(SmoothClientConfig::default());
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
    let mut host = av_host(SmoothClientConfig::default());
    host.route("Manifest", vec![Reply::Body(zero.into_bytes())]);
    host.run_until(|o| o.iter().any(|o| is_fatal(o) || is_ended(o)));
    assert!(!host.has(|o| matches!(o, Output::Tracks(_))));
    assert!(host.has(|o| matches!(
        o,
        Output::Fatal(SmoothError::Manifest(ManifestFault::Parse))
    )));
}
