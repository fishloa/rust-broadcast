//! Bug audit F: a push destination that accepts the connection and then drops
//! it must still be backed off and eventually given up on.

use std::time::Duration;

use multimux::config::ReconnectPolicy;
use multimux::push::ReconnectEngine;

#[test]
fn a_destination_that_connects_then_drops_hits_max_attempts() {
    let mut engine = ReconnectEngine::new(ReconnectPolicy {
        initial_backoff_ms: 1_000,
        max_backoff_ms: 30_000,
        max_attempts: Some(3),
    });
    for _ in 0..10 {
        engine.on_connect();
        engine.on_disconnect();
    }
    assert!(
        engine.is_failed(),
        "10 connect-then-drop cycles with max_attempts=3 never gave up"
    );
}

#[test]
fn a_destination_that_connects_then_drops_is_backed_off_progressively() {
    let mut engine = ReconnectEngine::new(ReconnectPolicy {
        initial_backoff_ms: 1_000,
        max_backoff_ms: 30_000,
        max_attempts: None,
    });
    for _ in 0..6 {
        engine.on_connect();
        engine.on_disconnect();
    }
    assert!(
        engine.time_until_retry() > Duration::from_secs(2),
        "after 6 connect-then-drop cycles the wait is still {:?}",
        engine.time_until_retry()
    );
}
