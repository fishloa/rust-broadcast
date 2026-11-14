//! SP1.5/#2471: the reconnect pacing multimux's inputs and outputs use is the
//! workspace's one schedule (broadcast-common's `RetrySchedule`) and one
//! reconnect rule (broadcast-client's `ReconnectTimer`), configured here with
//! this crate's production values.
//! `the_schedule_is_jittered_not_hand_arithmetic` FAILS on a hand-rolled
//! `powi` backoff (which returns exactly `raw` for every attempt —
//! deterministic and unjittered), and the cap-spread test FAILS on an
//! add-only jitter re-clamped at the cap (which is exactly `max` there).

use std::collections::HashSet;
use std::time::Duration;

use broadcast_common::backoff::RetrySchedule;
use multimux::config::ReconnectPolicy;
use multimux::push::ReconnectEngine;

/// A push output's backoff doubles on every attempt (the policy's documented
/// series, `RECONNECT_BACKOFF_FACTOR` in `multimux::config`).
/// The push output's schedule: the policy's `BackoffSteps` step table, drawn
/// as the shared [`RetrySchedule`].
fn schedule(policy: &ReconnectPolicy) -> RetrySchedule {
    policy
        .backoff
        .schedule(0x5EED)
        .expect("the policy's own bounds")
}

/// The same for the supervisor's production series (500 ms, 30 s, x2).
fn from_parts(min: Duration, max: Duration, factor: f64) -> RetrySchedule {
    RetrySchedule::new(min, max, factor, 0x5EED).expect("bounds a test chose")
}

fn seeded(min: Duration, max: Duration, factor: f64, seed: u64) -> RetrySchedule {
    RetrySchedule::new(min, max, factor, seed).expect("bounds a test chose")
}

/// The schedule is jittered, and it is the shared one: the first retry is
/// exactly the configured step (so a caller's first delay is what it
/// configured), every later delay is inside `[step*5/6, step]`, and a
/// different seed draws a different sequence.
///
/// Revert-check: a hand-rolled `raw * uniform[0.5, 1.0)` (what
/// `ReconnectSchedule::from_parts` produced before the shared schedule
/// existed) makes the first delay land below 10 s here.
#[test]
fn the_schedule_is_jittered_not_hand_arithmetic() {
    let a = from_parts(Duration::from_secs(10), Duration::from_secs(30), 2.0);
    let d0 = a.delay_for_attempt(0);
    assert_eq!(
        d0,
        Duration::from_secs(10),
        "the first retry is exactly the configured floor"
    );
    let b = seeded(
        Duration::from_secs(10),
        Duration::from_secs(30),
        2.0,
        0xDEAD_BEEF,
    );
    let seq_a: Vec<_> = (0..8).map(|i| a.delay_for_attempt(i)).collect();
    let seq_b: Vec<_> = (0..8).map(|i| b.delay_for_attempt(i)).collect();
    assert_ne!(
        seq_a, seq_b,
        "different seeds must jitter differently: {seq_a:?} vs {seq_b:?}"
    );
    let floor = Duration::from_secs(10).mul_f64(5.0 / 6.0);
    for i in 1..64 {
        let d = a.delay_for_attempt(i);
        assert!(
            d >= floor && d <= Duration::from_secs(30),
            "attempt {i}: jitter escaped [min*5/6, max]: {d:?}"
        );
    }
}

#[test]
fn the_schedule_doubles_from_the_min_and_never_exceeds_the_cap_even_with_jitter() {
    let s = from_parts(Duration::from_millis(500), Duration::from_secs(30), 2.0);
    for i in 0..64 {
        let d = s.delay_for_attempt(i);
        assert!(
            d <= Duration::from_secs(30),
            "attempt {i}: {d:?} must not exceed the cap"
        );
        assert!(
            d >= Duration::from_millis(416),
            "attempt {i}: {d:?} precedes the min*5/6 floor"
        );
    }
}

/// The delay must stay **jittered at the cap**, not collapse to exactly
/// `max`. A jitter added on top of the capped series and then re-clamped to
/// `max` yields exactly 30 s every time; the shared schedule's terms are
/// drawn after the cap, so the delay spreads below it.
///
/// Revert-check: re-clamp the jittered value to `max` and every sample is
/// exactly 30 s → `distinct == 1` → FAIL.
#[test]
fn the_delay_stays_jittered_at_the_cap() {
    let s = from_parts(Duration::from_millis(500), Duration::from_secs(30), 2.0);
    // Attempt 20 is deep in the saturated region (raw = cap).
    let mut distinct = HashSet::new();
    for _ in 0..64 {
        let d = s.delay_for_attempt(20);
        assert!(
            d >= Duration::from_secs(25) && d <= Duration::from_secs(30),
            "cap delay {d:?} must be jittered into [max*5/6, max]"
        );
        distinct.insert(d.as_nanos());
    }
    assert!(
        distinct.len() > 1,
        "the capped delay must not be a single constant (zero jitter): {distinct:?}"
    );
}

/// A seeded schedule is reproducible, and schedules built from different
/// entropy do not share a jitter sequence.
///
/// `mix_seed` is a pure avalanche (a caller supplies the entropy that must
/// differ, e.g. `SecureRandom::seed_entropy`), so the two "fresh" schedules
/// here are seeded the way `Backoff::production_default` seeds them: from
/// two different entropy values.
#[test]
fn seeded_schedules_are_reproducible_and_unseeded_ones_differ() {
    let a = seeded(Duration::from_millis(500), Duration::from_secs(30), 2.0, 7);
    let b = seeded(Duration::from_millis(500), Duration::from_secs(30), 2.0, 7);
    let seq_a: Vec<_> = (0..8).map(|i| a.delay_for_attempt(i)).collect();
    let seq_b: Vec<_> = (0..8).map(|i| b.delay_for_attempt(i)).collect();
    assert_eq!(
        seq_a, seq_b,
        "the same seed must reproduce the same sequence"
    );

    let x = seeded(
        Duration::from_millis(500),
        Duration::from_secs(30),
        2.0,
        broadcast_common::backoff::mix_seed(0),
    );
    let y = seeded(
        Duration::from_millis(500),
        Duration::from_secs(30),
        2.0,
        broadcast_common::backoff::mix_seed(1),
    );
    let sx: Vec<_> = (0..8).map(|i| x.delay_for_attempt(i)).collect();
    let sy: Vec<_> = (0..8).map(|i| y.delay_for_attempt(i)).collect();
    assert_ne!(
        sx, sy,
        "schedules from different entropy must not share a jitter sequence"
    );
}

#[test]
fn the_schedule_yields_attempts_past_a_short_attempt_bound() {
    // A schedule answers any attempt, however deep: attempt 10 is a real
    // (capped, jittered) delay, not an exhaustion sentinel.
    let s = from_parts(Duration::from_secs(1), Duration::from_secs(30), 2.0);
    let d = s.delay_for_attempt(10);
    assert!(
        d >= Duration::from_secs(25) && d <= Duration::from_secs(30),
        "attempt 10 must be a real capped+jittered delay: {d:?}"
    );
}

#[test]
fn a_policy_maps_to_the_same_min_and_cap() {
    // The push-output entry point: the policy's step table becomes the
    // schedule's floor and ceiling.
    let policy =
        ReconnectPolicy::doubling(Duration::from_millis(250), Duration::from_millis(4_000));
    let s = schedule(&policy);
    for i in 0..32 {
        let d = s.delay_for_attempt(i);
        assert!(
            d >= Duration::from_millis(208) && d <= Duration::from_millis(4_000),
            "attempt {i}: {d:?} outside the policy band"
        );
    }
}

/// m-1: a clone never duplicates the original's jitter sequence (Clone
/// reseeds).
#[test]
fn a_clone_reseeds_and_fresh_schedules_do_not_share_a_sequence() {
    let a = from_parts(Duration::from_millis(500), Duration::from_secs(30), 2.0);
    let b = a.clone();
    let seq_a: Vec<_> = (0..8).map(|i| a.delay_for_attempt(i)).collect();
    let seq_b: Vec<_> = (0..8).map(|i| b.delay_for_attempt(i)).collect();
    assert_ne!(
        seq_a, seq_b,
        "a clone must reseed, not replay the original's jitter sequence"
    );
}

#[test]
fn a_push_destination_that_connects_then_drops_hits_max_attempts() {
    let mut engine = ReconnectEngine::new(
        ReconnectPolicy::default().with_max_attempts(std::num::NonZeroU32::new(3).unwrap()),
    );
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
fn a_push_destination_that_connects_then_drops_is_backed_off_progressively() {
    let mut engine = ReconnectEngine::new(ReconnectPolicy::default());
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
