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
        ("max_playlist_bytes", |c| c.max_playlist_bytes = 0),
        ("max_segment_bytes", |c| c.max_segment_bytes = 0),
        ("max_key_bytes", |c| c.max_key_bytes = 0),
        ("hls_request_timeout_us", |c| c.hls_request_timeout_us = 0),
        ("hls_segment_timeout_us", |c| c.hls_segment_timeout_us = 0),
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
    let mut config = Config::default();
    config.max_queued_events = 1;
    config.max_playlist_bytes = 1;
    config.max_segment_bytes = 1;
    config.max_key_bytes = 1;
    config.hls_request_timeout_us = 1;
    config.hls_segment_timeout_us = 1;
    config.hls_key_timeout_us = 1;
    config.timeline_jump = Some(TimelineJumpConfig {
        backward_us: 1,
        forward_slack_us: 1,
    });
    assert_eq!(field_of(config), None);
}

#[test]
fn a_disabled_detector_and_unbounded_thresholds_open() {
    let mut config = Config::default();
    config.timeline_jump = None;
    assert_eq!(field_of(config.clone()), None);
    config.timeline_jump = Some(TimelineJumpConfig {
        backward_us: u64::MAX,
        forward_slack_us: u64::MAX,
    });
    assert_eq!(field_of(config), None);
}
