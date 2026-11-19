//! The `test-seams` fault-injection hooks (audit-22 #3008): a staged timer
//! error and a perpetually-overdue ("stuck") transport timer are real driver
//! states no live test can otherwise reach, so they must be exercised here
//! rather than assumed working.

#![cfg(all(feature = "media", feature = "test-seams"))]

use std::time::Instant;

use broadcast_common::stage::Timestamp;
use webrtc_runtime::error::MediaError;
use webrtc_runtime::media::{MediaEvent, MediaTransport, MediaTransportConfig, SetupRole};

/// A well-formed SHA-256 fingerprint (RFC 8122 §5) no test certificate
/// matches; the handshake is never expected to complete here.
const DUMMY_REMOTE_FINGERPRINT: &str = "sha-256 \
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:\
99:aa:bb:cc:dd:ee:ff";

/// The test's clock reading, a second past the transport's epoch.
fn now() -> Timestamp {
    Timestamp::from_micros(1_000_000)
}

fn transport() -> MediaTransport {
    MediaTransport::new(
        MediaTransportConfig::new(
            "127.0.0.1:41000".parse().expect("an address"),
            "localufrag0".into(),
            "local-ice-password-000000".into(),
            "remoteufrag0".into(),
            "remote-ice-password-000000".into(),
            SetupRole::Passive,
            DUMMY_REMOTE_FINGERPRINT.into(),
        ),
        now(),
    )
    .expect("a passive transport with a well-formed fingerprint")
}

fn timer_errors(events: Vec<MediaEvent>) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, MediaEvent::TimerError(_)))
        .count()
}

#[test]
fn a_staged_timer_error_is_reported_once_and_then_cleared() {
    let mut transport = transport();
    transport.force_next_timer_error(MediaError::Other("staged".into()));

    let first = transport.handle_timeout(now());
    assert_eq!(
        timer_errors(first),
        1,
        "the staged error must surface as one TimerError"
    );

    let second = transport.handle_timeout(now());
    assert_eq!(
        timer_errors(second),
        0,
        "the staged error is single-fire, not permanent"
    );
}

/// The `||` in `poll_timeout`'s staged-error check: a staged error on its own
/// (no stuck timer) must still make the next deadline immediate, or a driver
/// waiting for `poll_timeout` never reaches `handle_timeout`.
#[test]
fn a_staged_timer_error_schedules_an_immediate_deadline() {
    let mut transport = transport();
    transport.force_next_timer_error(MediaError::Other("staged".into()));

    let deadline = transport
        .poll_timeout()
        .expect("a staged timer error is due immediately");
    assert!(
        deadline <= Instant::now(),
        "a staged error must not wait for a real timer: {deadline:?}"
    );
}

#[test]
fn a_stuck_timer_reports_an_always_immediate_deadline() {
    let mut transport = transport();
    transport.force_stuck_timer();

    for _ in 0..8 {
        let deadline = transport
            .poll_timeout()
            .expect("a stuck timer is always due");
        assert!(
            deadline <= Instant::now(),
            "a stuck timer must report an immediate deadline: {deadline:?}"
        );
        let _ = transport.handle_timeout(now());
    }
}
