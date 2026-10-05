//! One reconnect schedule for every multimux retry, backed by `backon`
//! (SP1.5). Two verified `backon` 1.6.0 facts shape this wrapper
//! (`backon/src/backoff/exponential.rs`):
//!
//! 1. `ExponentialBuilder::new()` defaults `max_times: Some(3)` — an
//!    un-overridden builder stops yielding after 3 attempts, so every
//!    schedule here sets `without_max_times()` and applies its own bound.
//! 2. Jitter is ADDED after `backon`'s internal max-delay clamp
//!    (`tmp_cur.saturating_add(tmp_cur.mul_f32(rng.f32()))`), so a jittered
//!    delay can exceed `max_delay`; this wrapper re-clamps with `.min(max)`
//!    for a hard cap.
//!
//! Callers apply the returned [`Duration`] themselves (`tokio::time::sleep`
//! at the retry site), which is why `backon`'s own `tokio-sleep` feature is
//! off: a schedule is timing policy, not an executor.

use std::time::Duration;

use backon::BackoffBuilder as _;

/// A capped, jittered exponential retry schedule.
///
/// `delay_for_attempt(0)` is the delay before the FIRST retry. The raw series
/// is `min * factor^attempt`, capped at `max`; a random jitter of up to the
/// pre-jitter delay is then added, and the result is clamped back to `max`.
#[derive(Debug, Clone)]
pub struct ReconnectSchedule {
    builder: backon::ExponentialBuilder,
    max: Duration,
}

impl ReconnectSchedule {
    /// A schedule with a fresh random jitter seed.
    pub fn from_parts(min: Duration, max: Duration, factor: f64) -> Self {
        Self::from_parts_seeded(min, max, factor, random_seed())
    }

    /// A schedule with an explicit jitter seed (deterministic for tests).
    pub fn from_parts_seeded(min: Duration, max: Duration, factor: f64, seed: u64) -> Self {
        Self {
            // `with_jitter()` is REQUIRED: `backon` 1.6.0's builder defaults
            // `jitter: false` and `with_jitter_seed` only seeds the rng — it
            // does not enable jitter.
            builder: backon::ExponentialBuilder::new()
                .with_jitter()
                .with_jitter_seed(seed)
                .with_factor(factor as f32)
                .with_min_delay(min)
                .with_max_delay(max)
                .without_max_times(),
            max,
        }
    }

    /// The schedule a push output's [`ReconnectPolicy`](crate::config::ReconnectPolicy)
    /// describes.
    pub fn from_policy(policy: &crate::config::ReconnectPolicy) -> Self {
        Self::from_parts(
            Duration::from_millis(policy.initial_backoff_ms),
            Duration::from_millis(policy.max_backoff_ms),
            crate::config::RECONNECT_BACKOFF_FACTOR,
        )
    }

    /// The delay before the `attempt + 1`-th retry, bounded by `max` AFTER
    /// jitter.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt > MAX_ATTEMPT_INDEX {
            return self.max;
        }
        match self.builder.build().nth(attempt as usize) {
            Some(d) => d.min(self.max),
            None => self.max,
        }
    }
}

/// A non-cryptographic seed. `backon`'s jitter only needs to spread reconnect
/// storms across peers, not resist an adversary, and a wall-clock nanosecond
/// value is what `rand`'s own thread rng would seed from anyway.
fn random_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// The highest attempt whose delay is worth iterating for. `backon`'s
/// `ExponentialBackoff` is an `Iterator`, so `delay_for_attempt(u32::MAX)`
/// would step four billion times; well before this the raw series has
/// saturated at `max` (with any sane factor), so every later attempt returns
/// the cap without touching the iterator (matching the pre-SP1.5
/// `MAX_BACKOFF_EXPONENT` short-circuit).
const MAX_ATTEMPT_INDEX: u32 = 30;
