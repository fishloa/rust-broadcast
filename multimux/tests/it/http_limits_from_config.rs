//! #2948: `HttpLimits::from(&Config)` is the only path by which the
//! operator's concurrency, body-size and timeout knobs reach the server, so
//! every field is asserted against the config it came from.

use std::time::Duration;

use multimux::config::Config;
use multimux::origin::{
    DEFAULT_MAX_REQUEST_BODY_BYTES, DEFAULT_QUEUE_TIMEOUT, DEFAULT_REQUEST_TIMEOUT, HttpLimits,
};

fn config_from_json(json: &str) -> Config {
    serde_json::from_str(json).expect("a valid config")
}

#[test]
fn every_http_limit_comes_from_the_config() {
    let cfg = config_from_json(
        r#"{
            "request_timeout_secs": 7.0,
            "max_concurrent_requests": 3,
            "max_blocking_reloads_per_route": 5,
            "max_request_body_bytes": 123,
            "concurrency_queue_timeout_secs": 9.0
        }"#,
    );
    let limits = HttpLimits::from(&cfg);
    assert_eq!(limits.request_timeout, Duration::from_secs(7));
    assert_eq!(limits.max_concurrent_requests, 3);
    assert_eq!(limits.max_blocking_reloads_per_route, 5);
    assert_eq!(limits.max_request_body_bytes, 123);
    assert_eq!(limits.queue_timeout, Duration::from_secs(9));
}

#[test]
fn zero_timeouts_fall_back_to_the_documented_defaults() {
    let cfg = config_from_json(
        r#"{
            "request_timeout_secs": 0.0,
            "concurrency_queue_timeout_secs": 0.0
        }"#,
    );
    let limits = HttpLimits::from(&cfg);
    assert_eq!(limits.request_timeout, DEFAULT_REQUEST_TIMEOUT);
    assert_eq!(limits.request_timeout, Duration::from_secs(10));
    assert_eq!(limits.queue_timeout, DEFAULT_QUEUE_TIMEOUT);
    assert_eq!(limits.queue_timeout, Duration::from_secs(5));
    assert_eq!(DEFAULT_MAX_REQUEST_BODY_BYTES, 16_384);
}
