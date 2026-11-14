use broadcast_client::{Config, Error, Session, TimelineJumpConfig};
use broadcast_common::stage::Timestamp;
use buffering_runtime::micros_as_duration;
use url::Url;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const URL: &str = "http://h/live.ts";

fn field_of(config: Config) -> Option<&'static str> {
    match Session::open(Url::parse(URL).unwrap(), config, ts(0)) {
        Err(Error::InvalidConfig { field }) => Some(field),
        Err(other) => panic!("unexpected error {other}"),
        Ok(_) => None,
    }
}

#[test]
fn the_default_config_opens() {
    assert_eq!(field_of(Config::default()), None);
}

#[test]
fn every_numeric_field_that_would_silently_drop_output_is_rejected_by_name() {
    type Mutate = fn(&mut Config);
    let cases: [(&str, Mutate); 12] = [
        ("max_queued_events", |c| c.max_queued_events = 0),
        ("max_queued_bytes", |c| c.max_queued_bytes = 0),
        ("max_manifest_bytes", |c| c.max_manifest_bytes = 0),
        ("max_segment_bytes", |c| c.max_segment_bytes = 0),
        ("max_key_bytes", |c| c.max_key_bytes = 0),
        ("request_timeout", |c| {
            c.request_timeout = std::time::Duration::ZERO;
        }),
        ("segment_timeout", |c| {
            c.segment_timeout = std::time::Duration::ZERO;
        }),
        ("segment_inactivity_timeout", |c| {
            c.segment_inactivity_timeout = std::time::Duration::ZERO;
        }),
        ("hls_key_timeout", |c| {
            c.hls_key_timeout = std::time::Duration::ZERO;
        }),
        ("buffer", |c| {
            c.buffer.track_silence_timeout = micros_as_duration(0);
        }),
        ("timeline_jump.backward", |c| {
            c.timeline_jump =
                Some(TimelineJumpConfig::default().with_backward(std::time::Duration::ZERO));
        }),
        ("timeline_jump.forward_slack", |c| {
            c.timeline_jump =
                Some(TimelineJumpConfig::default().with_forward_slack(std::time::Duration::ZERO));
        }),
    ];
    for (field, mutate) in cases {
        let mut config = Config::default();
        mutate(&mut config);
        assert_eq!(field_of(config), Some(field), "{field}");
    }
}

#[test]
fn the_smallest_valid_values_open() {
    let config = {
        let mut new_config = Config::default();
        new_config.max_queued_events = 1;
        new_config.max_queued_bytes = 1;
        new_config.max_manifest_bytes = 1;
        new_config.max_segment_bytes = 1;
        new_config.max_key_bytes = 1;
        new_config.request_timeout = std::time::Duration::from_micros(1);
        new_config.segment_timeout = std::time::Duration::from_micros(1);
        new_config.segment_inactivity_timeout = std::time::Duration::from_micros(1);
        new_config.hls_key_timeout = std::time::Duration::from_micros(1);
        new_config.timeline_jump = Some(
            TimelineJumpConfig::default()
                .with_backward(std::time::Duration::from_micros(1))
                .with_forward_slack(std::time::Duration::from_micros(1)),
        );
        new_config
    };
    assert_eq!(field_of(config), None);
}

#[test]
fn a_disabled_detector_and_unbounded_thresholds_open() {
    let disabled = {
        let mut new_config = Config::default();
        new_config.timeline_jump = None;
        new_config
    };
    assert_eq!(field_of(disabled), None);
    let unbounded = {
        let mut new_config = Config::default();
        new_config.timeline_jump = Some(
            TimelineJumpConfig::default()
                .with_backward(std::time::Duration::MAX)
                .with_forward_slack(std::time::Duration::MAX),
        );
        new_config
    };
    assert_eq!(field_of(unbounded), None);
}

const RETIRED_KNOBS: [&str; 10] = [
    "hls_backoff_ms",
    "hls_backoff_cap_ms",
    "hls_max_segment_failures",
    "hls_max_inflight_segments",
    "hls_request_timeout_us",
    "hls_segment_timeout_us",
    "max_playlist_bytes",
    "DEFAULT_MAX_PLAYLIST_BYTES",
    "DEFAULT_HLS_BACKOFF_MS",
    "DEFAULT_HLS_MAX_SEGMENT_FAILURES",
];

fn rust_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn retired_hls_prefixed_knob_names_do_not_reappear_in_src() {
    let mut files = Vec::new();
    rust_sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(!files.is_empty());
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read");
        for name in RETIRED_KNOBS {
            assert!(!text.contains(name), "{} mentions {name}", file.display());
        }
    }
}
