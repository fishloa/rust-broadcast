//! SP1.5: one `backon`-backed schedule with an explicit attempt bound and a
//! jitter-bounded cap. `the_schedule_is_jittered_not_hand_arithmetic` FAILS on
//! the old hand-rolled `powi` backoff (which returns exactly `raw` for every
//! attempt — deterministic and unjittered), and the cap-spread test FAILS on
//! the old add-only `backon` jitter (which is exactly `max` at the cap).

use std::collections::HashSet;
use std::time::Duration;

use multimux::reconnect::ReconnectSchedule;

#[test]
fn the_schedule_is_jittered_not_hand_arithmetic() {
    // The old `powi` code returns exactly the raw value for every attempt —
    // deterministic and unjittered. Equal jitter multiplies the raw delay by
    // `uniform[0.5, 1.0)`, so attempt 0 (raw = min = 10 s) lands in
    // `[5 s, 10 s)`.
    let a = ReconnectSchedule::from_parts(Duration::from_secs(10), Duration::from_secs(30), 2.0);
    let d0 = a.delay_for_attempt(0);
    assert!(
        d0 >= Duration::from_secs(5) && d0 < Duration::from_secs(10),
        "attempt 0 must be equal-jittered into [min/2, min): got {d0:?}"
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
    // Every delay stays within `[min/2, max)`.
    for i in 0..64 {
        let d = a.delay_for_attempt(i);
        assert!(
            d >= Duration::from_secs(5) && d < Duration::from_secs(30),
            "attempt {i}: jitter escaped [min/2, max): {d:?}"
        );
    }
}

#[test]
fn the_schedule_doubles_from_the_min_and_never_exceeds_the_cap_even_with_jitter() {
    let s = ReconnectSchedule::from_parts(Duration::from_millis(500), Duration::from_secs(30), 2.0);
    for i in 0..64 {
        let d = s.delay_for_attempt(i);
        assert!(
            d < Duration::from_secs(30),
            "attempt {i}: {d:?} must stay below the cap"
        );
        assert!(
            d >= Duration::from_millis(250),
            "attempt {i}: {d:?} precedes the min/2 floor"
        );
    }
}

/// C2: the delay must stay **jittered at the cap**, not collapse to exactly
/// `max`. With raw saturating at 30 s, an add-only jitter (the old shape)
/// yields exactly 30 s every time; equal jitter yields a spread in `[15 s, 30 s)`.
///
/// Revert-check: restore the add-only `backon` jitter (`.with_jitter().min(max)`)
/// and every sample is exactly 30 s → `distinct == 1` → FAIL.
#[test]
fn the_delay_stays_jittered_at_the_cap() {
    let s = ReconnectSchedule::from_parts(Duration::from_millis(500), Duration::from_secs(30), 2.0);
    // Attempt 20 is deep in the saturated region (raw = cap).
    let mut distinct = HashSet::new();
    for _ in 0..64 {
        let d = s.delay_for_attempt(20);
        assert!(
            d >= Duration::from_secs(15) && d < Duration::from_secs(30),
            "cap delay {d:?} must be equal-jittered into [max/2, max)"
        );
        distinct.insert(d.as_nanos());
    }
    assert!(
        distinct.len() > 1,
        "the capped delay must not be a single constant (zero jitter): {distinct:?}"
    );
}

/// C2: a seeded schedule is reproducible, and two schedules made in the same
/// coarse clock tick do not share a jitter sequence (per-process counter seed
/// mixing).
#[test]
fn seeded_schedules_are_reproducible_and_unseeded_ones_differ() {
    let a = ReconnectSchedule::from_parts_seeded(
        Duration::from_millis(500),
        Duration::from_secs(30),
        2.0,
        7,
    );
    let b = ReconnectSchedule::from_parts_seeded(
        Duration::from_millis(500),
        Duration::from_secs(30),
        2.0,
        7,
    );
    let seq_a: Vec<_> = (0..8).map(|i| a.delay_for_attempt(i)).collect();
    let seq_b: Vec<_> = (0..8).map(|i| b.delay_for_attempt(i)).collect();
    assert_eq!(
        seq_a, seq_b,
        "the same seed must reproduce the same sequence"
    );

    // Two independently-seeded schedules created back-to-back must not be
    // identical (the counter mix defeats a shared coarse-tick seed).
    let x = ReconnectSchedule::from_parts(Duration::from_millis(500), Duration::from_secs(30), 2.0);
    let y = ReconnectSchedule::from_parts(Duration::from_millis(500), Duration::from_secs(30), 2.0);
    let sx: Vec<_> = (0..8).map(|i| x.delay_for_attempt(i)).collect();
    let sy: Vec<_> = (0..8).map(|i| y.delay_for_attempt(i)).collect();
    assert_ne!(
        sx, sy,
        "two fresh schedules must not share a jitter sequence"
    );
}

#[test]
fn the_schedule_yields_attempts_past_backons_default_three() {
    // `backon`'s default `max_times` is 3 (`ExponentialBuilder::new`,
    // backon 1.6.0 exponential.rs) — the schedule must override it or the
    // 4th retry would return the cap-sentinel. Ask for attempt 10 and get a
    // real (capped) delay below the cap, not the exhaustion fallback.
    let s = ReconnectSchedule::from_parts(Duration::from_secs(1), Duration::from_secs(30), 2.0);
    let d = s.delay_for_attempt(10);
    assert!(
        d >= Duration::from_secs(15) && d < Duration::from_secs(30),
        "attempt 10 must be a real capped+equal-jittered delay: {d:?}"
    );
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
            d >= Duration::from_millis(125) && d < Duration::from_millis(4_000),
            "attempt {i}: {d:?} outside the policy band"
        );
    }
}
