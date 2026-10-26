use broadcast_client::{Config, Error, Session, TimelineJumpConfig};

const URL: &str = "http://h/live.ts";

fn field_of(config: Config) -> Option<&'static str> {
    match Session::open(URL, config, 0) {
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
    let cases: [(&str, Mutate); 10] = [
        ("max_queued_events", |c| c.max_queued_events = 0),
        ("max_manifest_bytes", |c| c.max_manifest_bytes = 0),
        ("max_segment_bytes", |c| c.max_segment_bytes = 0),
        ("max_key_bytes", |c| c.max_key_bytes = 0),
        ("request_timeout_us", |c| c.request_timeout_us = 0),
        ("segment_timeout_us", |c| c.segment_timeout_us = 0),
        ("hls_key_timeout_us", |c| c.hls_key_timeout_us = 0),
        ("buffer", |c| c.buffer.track_silence_timeout_us = 0),
        ("timeline_jump.backward_us", |c| {
            c.timeline_jump = Some(TimelineJumpConfig {
                backward_us: 0,
                ..TimelineJumpConfig::default()
            });
        }),
        ("timeline_jump.forward_slack_us", |c| {
            c.timeline_jump = Some(TimelineJumpConfig {
                forward_slack_us: 0,
                ..TimelineJumpConfig::default()
            });
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
    let config = Config {
        max_queued_events: 1,
        max_manifest_bytes: 1,
        max_segment_bytes: 1,
        max_key_bytes: 1,
        request_timeout_us: 1,
        segment_timeout_us: 1,
        hls_key_timeout_us: 1,
        timeline_jump: Some(TimelineJumpConfig {
            backward_us: 1,
            forward_slack_us: 1,
        }),
        ..Config::default()
    };
    assert_eq!(field_of(config), None);
}

#[test]
fn a_disabled_detector_and_unbounded_thresholds_open() {
    let disabled = Config {
        timeline_jump: None,
        ..Config::default()
    };
    assert_eq!(field_of(disabled), None);
    let unbounded = Config {
        timeline_jump: Some(TimelineJumpConfig {
            backward_us: u64::MAX,
            forward_slack_us: u64::MAX,
        }),
        ..Config::default()
    };
    assert_eq!(field_of(unbounded), None);
}
