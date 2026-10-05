//! One reconnect schedule for every multimux retry, backed by `backon`
//! (SP1.5). Verified `backon` 1.6.0 facts that shape this wrapper
//! (`backon/src/backoff/exponential.rs`):
//!
//! 1. `ExponentialBuilder::new()` defaults `max_times: Some(3)` — an
//!    un-overridden builder stops yielding after 3 attempts, so every
//!    schedule here sets `without_max_times()` and applies its own bound.
//! 2. `backon`'s built-in jitter is **add-only and applied after its own
//!    max-delay clamp** (`tmp_cur.saturating_add(tmp_cur.mul_f32(rng.f32()))`),
//!    so at the cap it produces `[max, 2*max)` clamped back to exactly `max`
//!    — i.e. **zero jitter in steady state**, which is where a long outage
//!    lives. This wrapper therefore disables `backon`'s jitter and applies
//!    its own **equal jitter** to the raw capped series.
//!
//! # Equal jitter
//!
//! For a raw delay `raw` (the capped exponential), the returned delay is
//! `raw * uniform[0.5, 1.0)`. This keeps a non-degenerate spread at *every*
//! attempt, including the cap (`[max/2, max)`), so a fleet of routes pointed
//! at one dead server never retries in lockstep — the thundering herd SP1.5
//! exists to remove. It also never exceeds `max`, so no re-clamp is needed.
//!
//! Callers apply the returned [`Duration`] themselves (`tokio::time::sleep`
//! at the retry site), which is why `backon`'s own `tokio-sleep` feature is
//! off: a schedule is timing policy, not an executor.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use backon::BackoffBuilder as _;

/// A capped, equal-jittered exponential retry schedule.
///
/// `delay_for_attempt(0)` is the delay before the FIRST retry. The raw series
/// is `min * factor^attempt`, capped at `max`; the returned delay is
/// `raw * uniform[0.5, 1.0)`, so it is always in `[raw/2, raw)` and therefore
/// within `[min/2, max)`.
/// # Concurrency
///
/// The jitter state is a `Cell<u64>`, so a `ReconnectSchedule` is **not
/// `Sync`** (and `delay_for_attempt(&self)` mutates it — it is not a pure
/// read). It is meant to be owned by one task. `Clone` is implemented by hand
/// to **reseed** rather than duplicate the RNG state, so a clone can never
/// replay the original's jitter sequence.
#[derive(Debug)]
pub struct ReconnectSchedule {
    /// Produces the *raw* capped series (jitter disabled).
    builder: backon::ExponentialBuilder,
    min: Duration,
    max: Duration,
    /// Per-schedule jitter state: a 64-bit xorshift seeded from the caller's
    /// seed (see [`random_seed`]). Cheap, dependency-free, and repr-independent
    /// of `backon`'s own RNG.
    rng: std::cell::Cell<u64>,
}

impl ReconnectSchedule {
    /// A schedule with a fresh random jitter seed.
    pub fn from_parts(min: Duration, max: Duration, factor: f64) -> Self {
        Self::from_parts_seeded(min, max, factor, random_seed())
    }

    /// A schedule with an explicit jitter seed (deterministic for tests).
    pub fn from_parts_seeded(min: Duration, max: Duration, factor: f64, seed: u64) -> Self {
        Self {
            // Jitter is DISABLED here — this wrapper applies equal jitter
            // itself (see the module doc); `backon` is used only for its raw
            // capped exponential series and its `without_max_times` flag.
            builder: backon::ExponentialBuilder::new()
                .with_factor(factor as f32)
                .with_min_delay(min)
                .with_max_delay(max)
                .without_max_times(),
            min,
            max,
            // A zero seed would make xorshift a fixed point; or in the low
            // bit so it never is.
            rng: std::cell::Cell::new(seed | 1),
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

    /// The next `uniform[0, 1)` draw from this schedule's xorshift state.
    fn next_unit(&self) -> f32 {
        // xorshift64; the low bits of the multiply-shift are fine for a
        // timing jitter that needs no cryptographic quality.
        let mut x = self.rng.get();
        x ^= x << XORSHIFT_LEFT_A;
        x ^= x >> XORSHIFT_RIGHT_B;
        x ^= x << XORSHIFT_LEFT_C;
        self.rng.set(x);
        // `UNIT_FRACTION_BITS` HIGH bits of the 64-bit state as a float in
        // [0, 1): shift right by `64 - UNIT_FRACTION_BITS`.
        let shift = u64::BITS - UNIT_FRACTION_BITS;
        ((x >> shift) as f32) / (UNIT_FRACTION_SCALE as f32)
    }

    /// The raw capped exponential delay for `attempt` (no jitter).
    fn raw_delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt > MAX_ATTEMPT_INDEX {
            return self.max;
        }
        match self.builder.build().nth(attempt as usize) {
            Some(d) => d.min(self.max),
            None => self.max,
        }
    }

    /// The delay before the `attempt + 1`-th retry: the raw capped series
    /// (`min * factor^attempt`, capped at `max`) times `uniform[0.5, 1.0)`, so
    /// the result is always in `[raw/2, raw) ⊆ [min/2, max)`.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let raw = self.raw_delay_for_attempt(attempt);
        let factor = JITTER_LOW + (1.0 - JITTER_LOW) * f64::from(self.next_unit());
        // `raw` is at most `max`, so `raw * factor < max` always; the `min`
        // guards a `raw` of zero.
        raw.mul_f64(factor).max(self.min / 2)
    }
}

impl Clone for ReconnectSchedule {
    /// A clone with a *fresh* jitter seed (never a duplicate sequence).
    fn clone(&self) -> Self {
        Self {
            builder: self.builder,
            min: self.min,
            max: self.max,
            rng: std::cell::Cell::new(random_seed() | 1),
        }
    }
}

/// SplitMix64 constants (Steele et al. 2014): the golden-ratio increment, the
/// two 64-bit multipliers, and the three avalanche shift amounts.
const SPLITMIX_GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
const SPLITMIX_MUL_A: u64 = 0xBF58_476D_1CE4_E5B9;
const SPLITMIX_MUL_B: u64 = 0x94D0_49BB_1331_11EB;
const SPLITMIX_SHIFT_A: u32 = 30;
const SPLITMIX_SHIFT_B: u32 = 27;
const SPLITMIX_SHIFT_C: u32 = 31;

/// xorshift64 shift triple (Marsaglia 2003): 13 left, 7 right, 17 left.
const XORSHIFT_LEFT_A: u32 = 13;
const XORSHIFT_RIGHT_B: u32 = 7;
const XORSHIFT_LEFT_C: u32 = 17;
/// How many high bits of the xorshift state become the unit fraction.
const UNIT_FRACTION_BITS: u32 = 24;
/// `1 << UNIT_FRACTION_BITS` — the divisor that turns those bits into `[0,1)`.
const UNIT_FRACTION_SCALE: u32 = 1u32 << UNIT_FRACTION_BITS;
/// Equal-jitter band: multiply the raw delay by `uniform[JITTER_LOW, 1.0)`.
const JITTER_LOW: f64 = 0.5;

/// A non-cryptographic seed, mixed with a per-process counter so two
/// schedules created in the same coarse clock tick (a route loop spawning
/// many routes, or macOS's low `SystemTime` granularity) do not share a seed
/// and therefore an identical jitter sequence.
fn random_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // SplitMix64 avalanche of the tick ^ a monotonically increasing counter
    // (Steele et al. 2014): a golden-ratio increment, then three shift-xor-
    // multiply rounds.
    let mut z = nanos ^ COUNTER.fetch_add(SPLITMIX_GOLDEN, Ordering::Relaxed);
    z = (z ^ (z >> SPLITMIX_SHIFT_A)).wrapping_mul(SPLITMIX_MUL_A);
    z = (z ^ (z >> SPLITMIX_SHIFT_B)).wrapping_mul(SPLITMIX_MUL_B);
    z ^ (z >> SPLITMIX_SHIFT_C)
}

/// The highest attempt whose delay is worth iterating for. `backon`'s
/// `ExponentialBackoff` is an `Iterator`, so iterating for `u32::MAX` would
/// step four billion times; well before this the raw series has saturated at
/// `max` (with any sane factor), so every later attempt returns the cap
/// without touching the iterator (matching the pre-SP1.5
/// `MAX_BACKOFF_EXPONENT` short-circuit).
const MAX_ATTEMPT_INDEX: u32 = 30;
