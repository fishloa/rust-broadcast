//! SP1.5: one `backon`-backed schedule with an explicit attempt bound and a
//! jitter-proof cap. `the_schedule_uses_backon_not_hand_arithmetic` FAILS on
//! the old hand-rolled `powi` backoff (which returns exactly `min` for
//! attempt 0 — deterministic and unjittered).

use std::time::Duration;

use multimux::reconnect::ReconnectSchedule;

#[test]
fn the_schedule_uses_backon_not_hand_arithmetic() {
    // The old `powi` code returns exactly `min` for attempt 0 —
    // deterministic and unjittered — so the jitter-band assertions fail on
    // it. `backon`'s jitter adds `(0, current_delay)` to the pre-jitter
    // delay, so attempt 0 lands strictly inside `[min, 2*min)`.
    let a = ReconnectSchedule::from_parts(Duration::from_secs(10), Duration::from_secs(30), 2.0);
    let d0 = a.delay_for_attempt(0);
    assert!(
        d0 > Duration::from_secs(10) && d0 < Duration::from_secs(20),
        "attempt 0 must be jittered into (min, 2*min): got {d0:?}"
    );
    let b = ReconnectSchedule::from_parts_seeded(
        Duration::from_secs(10),
        Duration::from_secs(30),
        2.0,
        0xDEAD_BEEF,
    );
    let d0b = b.delay_for_attempt(0);
    assert_ne!(
        d0, d0b,
        "different seeds must jitter differently: {d0:?} vs {d0b:?}"
    );
    // And every jittered delay stays within `[min, cap]` (the re-clamp).
    for i in 0..64 {
        let d = a.delay_for_attempt(i);
        assert!(
            d >= Duration::from_secs(10) && d <= Duration::from_secs(30),
            "attempt {i}: jitter escaped [min, cap]: {d:?}"
        );
    }
}

#[test]
fn the_schedule_doubles_from_the_min_and_never_exceeds_the_cap_even_with_jitter() {
    let s = ReconnectSchedule::from_parts(Duration::from_millis(500), Duration::from_secs(30), 2.0);
    let mut d = Duration::ZERO;
    for i in 0..64 {
        d = s.delay_for_attempt(i);
        assert!(
            d <= Duration::from_secs(30),
            "attempt {i}: {d:?} exceeds the cap"
        );
        assert!(
            d >= Duration::from_millis(500),
            "attempt {i}: {d:?} precedes the min"
        );
    }
    // Once the raw delay saturates at the cap, every jittered+re-clamped
    // value is exactly the cap.
    assert_eq!(d, Duration::from_secs(30));
}

#[test]
fn the_schedule_yields_attempts_past_backons_default_three() {
    // `backon`'s default `max_times` is 3 (`ExponentialBuilder::new`,
    // backon 1.6.0 exponential.rs) — the schedule must override it or the
    // 4th retry would return the cap-sentinel. Ask for attempt 10 and get a
    // real (capped) delay, not the exhaustion fallback.
    let s = ReconnectSchedule::from_parts(Duration::from_secs(1), Duration::from_secs(30), 2.0);
    assert_eq!(s.delay_for_attempt(10), Duration::from_secs(30));
}

#[test]
fn a_policy_maps_to_the_same_min_and_cap() {
    // `ReconnectSchedule::from_policy` is the push-output entry point: the
    // policy's millisecond bounds become the schedule's floor and ceiling.
    use multimux::config::ReconnectPolicy;
    let policy = ReconnectPolicy {
        initial_backoff_ms: 250,
        max_backoff_ms: 4_000,
        max_attempts: None,
    };
    let s = ReconnectSchedule::from_policy(&policy);
    for i in 0..32 {
        let d = s.delay_for_attempt(i);
        assert!(
            d >= Duration::from_millis(250) && d <= Duration::from_millis(4_000),
            "attempt {i}: {d:?} outside the policy band"
        );
    }
}
