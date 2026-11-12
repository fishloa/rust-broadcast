//! One capped, equal-jittered retry schedule for every caller that retries on
//! a timer (issue #2471: the workspace carried five-plus copies).
//!
//! It is `no_std` and uses nothing but integer arithmetic plus `f64`
//! multiply: the workspace's `thumbv7em-none-eabi` build needs no float
//! library for it.
//!
//! A schedule is pure policy: it answers "how long before attempt `n`?" and
//! never sleeps, reads a clock, or allocates. The caller owns the timer
//! (`tokio::time::sleep`, a TIMERFD, a driver's own wake-up) and the clock
//! (`Instant::now`), which is what keeps every consumer of this module
//! clock-free and testable against a fixed seed.
//!
//! # Equal jitter
//!
//! The raw series is `min * factor^attempt`, capped at `max`. The first retry
//! is the raw delay itself, so a caller gets exactly the step it configured;
//! every later delay is `raw` times a term of `[5/6, 1.0)`, so it stays within
//! `[raw*5/6, raw]`: never above the configured step, never more than a sixth
//! below it.
//!
//! Jitter is applied *after* the cap deliberately: a jitter that is added on
//! top and then re-clamped to `max` (the shape `backon`'s `with_jitter`
//! produces) yields exactly `max` at the cap, i.e. no spread at all — and the
//! cap is where a long outage lives, so that is precisely the case a fleet of
//! clients pointed at one dead server must not lockstep on.
//!
//! The jittered terms of one schedule form a fixed sequence: the first is the
//! raw delay itself, and each later term is a fixed point of the jitter draw.
//! A client's attempts therefore walk that sequence instead of re-rolling it
//! (a series re-rolled per attempt clusters its terms), and a caller may start
//! at any offset of it — which is how a fleet spread over the retry interval
//! stays spread.
//!
//! # Seeding
//!
//! The jitter terms are drawn from the caller's seed through one SplitMix64
//! avalanche ([`mix_seed`]), so adjacent seeds give unrelated sequences. No
//! clock and no global counter is read here: a caller that wants two processes
//! to differ supplies entropy that differs.
//!
//! ```rust
//! use std::time::Duration;
//! use broadcast_common::backoff::RetrySchedule;
//!
//! let schedule = RetrySchedule::new(
//!     Duration::from_millis(500),
//!     Duration::from_secs(30),
//!     2.0,
//!     0x5EED,
//! );
//! for attempt in 0..8 {
//!     let delay = schedule.delay_for_attempt(attempt);
//!     assert!(delay >= schedule.raw_delay_for_attempt(attempt) * 5 / 6);
//!     assert!(delay <= schedule.raw_delay_for_attempt(attempt));
//! }
//! ```

use core::cell::Cell;
use core::fmt;
use core::time::Duration;

/// The highest attempt [`RetrySchedule::raw_delay_for_attempt`] grows the
/// series to; well before it the raw delay has saturated at the cap for any
/// sane factor, so every later attempt returns the cap without iterating.
const MAX_ATTEMPT_INDEX: u32 = 30;

/// xorshift64 shift triple (Marsaglia 2003).
const XORSHIFT_LEFT_A: u32 = 13;
const XORSHIFT_RIGHT_B: u32 = 7;
const XORSHIFT_LEFT_C: u32 = 17;

/// How many high bits of the xorshift state become the unit fraction.
const UNIT_FRACTION_BITS: u32 = 24;
/// `1 << UNIT_FRACTION_BITS`, the divisor turning those bits into `[0, 1)`.
const UNIT_FRACTION_SCALE: u32 = 1 << UNIT_FRACTION_BITS;

/// The band's width as a fraction of the raw delay: every jittered term is at most a sixth from
/// the raw delay, so each stays in `[raw*5/6, raw)`.
///
/// A sixth rather than a half because the jitter exists to break lockstep, not
/// to shorten a schedule: a delay that may come in at half of the configured
/// step is a different (twice as aggressive) policy from the one every table in
/// the workspace asks for.
const JITTER_SPREAD: f64 = 1.0 / 6.0;

/// The lower edge of a jittered term: `raw * (1 - JITTER_SPREAD)`.
const JITTER_LOW: f64 = 1.0 - JITTER_SPREAD;

/// How many jittered terms one schedule's sequence holds (see
/// [`RetrySchedule::delay_for_attempt`]): enough for a fleet spread over the retry interval to
/// map onto distinct terms, and short enough to keep a schedule small.
const JITTER_TERMS: usize = 16;

/// A growth factor that leaves the raw series at its floor.
const NO_GROWTH: f64 = 1.0;

/// The attempt index of the first retry (and of the sequence's first term).
const FIRST_RETRY: u32 = 0;

/// The sequence's first term: the raw delay itself, so a caller's first retry is exactly the
/// step it configured.
const FIRST_TERM: usize = FIRST_RETRY as usize;

/// A capped, equal-jittered retry schedule.
///
/// [`delay_for_attempt`](Self::delay_for_attempt) is the jittered delay the
/// caller waits; [`raw_delay_for_attempt`](Self::raw_delay_for_attempt) is the
/// deterministic series it is drawn from.
///
/// # Concurrency
///
/// The jitter state is a `Cell`, so a schedule is `Send` but not `Sync`, and
/// `delay_for_attempt` takes `&self` while advancing it. One schedule belongs
/// to one task. [`Clone`] reseeds rather than duplicating the state, so a
/// clone can never replay its original's jitter sequence.
pub struct RetrySchedule {
    kind: Kind,
    max: Duration,
    terms: [f64; JITTER_TERMS],
    cursor: Cell<usize>,
}

/// How the raw (unjittered) series is produced.
#[derive(Clone)]
enum Kind {
    /// `min * factor^attempt`, capped at `max`.
    Exponential { min: Duration, factor: f64 },
    /// A literal per-attempt step table, then `max` for every later attempt.
    Steps(Steps),
}

/// A literal step table (see [`RetrySchedule::from_steps`]).
#[derive(Clone)]
struct Steps {
    delays: alloc::vec::Vec<Duration>,
}

impl RetrySchedule {
    /// A schedule from `min` (the first raw delay), `max` (the cap), a growth
    /// factor and a jitter seed.
    ///
    /// A `factor` below one is raised to [`NO_GROWTH`] so the series can never
    /// shrink, and a `max` below `min` is raised to `min` so the cap is never
    /// tighter than the floor.
    #[must_use]
    pub fn new(min: Duration, max: Duration, factor: f64, seed: u64) -> Self {
        Self {
            kind: Kind::Exponential {
                min,
                factor: if factor >= NO_GROWTH {
                    factor
                } else {
                    NO_GROWTH
                },
            },
            max: max.max(min),
            terms: jitter_terms(seed),
            cursor: Cell::new(FIRST_TERM + 1),
        }
    }

    /// A schedule whose raw series is the literal table `steps` (the first entry is the first
    /// retry's delay), then `cap` for every later attempt, jittered the same way.
    ///
    /// This is the shape a caller with a hand-chosen step table has, kept distinct from
    /// [`Self::new`] so an existing table becomes configuration of one schedule rather than being
    /// re-derived as an exponential series that only happens to pass through the same numbers.
    /// The table is kept as given (a step may exceed `cap`; it is never the schedule's cap, which
    /// only bounds the *exponential* series); an empty table is the cap from the first attempt.
    #[must_use]
    pub fn from_steps(steps: &[Duration], cap: Duration, seed: u64) -> Self {
        Self {
            kind: Kind::Steps(Steps {
                delays: steps.to_vec(),
            }),
            max: cap,
            terms: jitter_terms(seed),
            cursor: Cell::new(FIRST_TERM + 1),
        }
    }

    /// The same schedule with the retry that has been cancelled starting at `offset` of its
    /// jitter sequence instead of at the top.
    ///
    /// A fleet spread over the retry interval can map each client to its own offset into the
    /// sequence, so the retry band `[raw*5/6, raw)` stays as spread as the instant it replaced
    /// rather than collapsing onto one term. Offset `0` is the schedule as built (the first
    /// retry is exactly the raw delay).
    #[must_use]
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.cursor = Cell::new(offset % JITTER_TERMS);
        self
    }

    /// The delay before the `attempt + 1`-th retry.
    ///
    /// The first retry takes the raw delay itself; every later one takes the next fixed point of
    /// the schedule's jitter sequence (wrapping at [`JITTER_TERMS`]), each in `[raw*5/6, raw]`.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let raw = self.raw_delay_for_attempt(attempt);
        if attempt == FIRST_RETRY || attempt > MAX_ATTEMPT_INDEX {
            return raw;
        }
        let cursor = self.cursor.get() % JITTER_TERMS;
        self.cursor.set((cursor + 1) % JITTER_TERMS);
        scaled(raw, term_power(self.terms[cursor], attempt))
    }

    /// The delay before the `attempt + 1`-th retry without jitter:
    /// `min * factor^attempt`, capped at `max`, or the literal step table's
    /// entry. The deterministic bound the jittered delay never exceeds.
    #[must_use]
    pub fn raw_delay_for_attempt(&self, attempt: u32) -> Duration {
        match &self.kind {
            Kind::Steps(steps) => steps
                .delays
                .get(usize::try_from(attempt).unwrap_or(usize::MAX))
                .copied()
                .unwrap_or(self.max),
            Kind::Exponential { min, factor } => {
                if attempt > MAX_ATTEMPT_INDEX {
                    return self.max;
                }
                // `attempt` is bounded by `MAX_ATTEMPT_INDEX` above.
                min.mul_f64(term_power(*factor, attempt)).min(self.max)
            }
        }
    }

    /// The first (unjittered) delay of the series.
    #[must_use]
    pub fn min(&self) -> Duration {
        match &self.kind {
            Kind::Steps(steps) => steps.delays.first().copied().unwrap_or(self.max),
            Kind::Exponential { min, .. } => *min,
        }
    }

    /// The cap the series saturates at.
    #[must_use]
    pub fn max(&self) -> Duration {
        self.max
    }
}

impl Clone for RetrySchedule {
    /// A clone with a *fresh* seed, never a duplicate jitter sequence.
    fn clone(&self) -> Self {
        Self {
            kind: self.kind.clone(),
            max: self.max,
            terms: jitter_terms(mix_seed(self.min().as_nanos() as u64)),
            cursor: Cell::new(FIRST_TERM + 1),
        }
    }
}

impl fmt::Debug for RetrySchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetrySchedule")
            .field("min", &self.min())
            .field("max", &self.max)
            .finish_non_exhaustive()
    }
}

/// The schedule's jitter sequence: [`JITTER_TERMS`] fixed points of the equal-jitter draw, the
/// first of which is exactly one (the raw delay itself).
///
/// A term is `q**attempt` for an attempt, so a term of `1` is what makes the first retry the
/// configured step, and the terms are drawn from a seed avalanched up front so that adjacent
/// caller seeds (1, 2, 3) do not share a low-entropy start.
fn jitter_terms(seed: u64) -> [f64; JITTER_TERMS] {
    let mut terms = [1.0; JITTER_TERMS];
    let mut state = avalanche(seed);
    for term in &mut terms[FIRST_TERM + 1..] {
        state ^= state << XORSHIFT_LEFT_A;
        state ^= state >> XORSHIFT_RIGHT_B;
        state ^= state << XORSHIFT_LEFT_C;
        let shift = u64::BITS - UNIT_FRACTION_BITS;
        let unit = f64::from((state >> shift) as u32) / f64::from(UNIT_FRACTION_SCALE);
        *term = JITTER_LOW + JITTER_SPREAD * unit;
    }
    terms
}

/// `term` raised to `attempt`, multiplied out one factor at a time: `core` has no `powi`, and
/// `attempt` is bounded by [`MAX_ATTEMPT_INDEX`].
fn term_power(term: f64, attempt: u32) -> f64 {
    let mut power = 1.0;
    for _ in 0..attempt {
        power *= term;
    }
    power
}

/// `raw * factor`, kept inside `[raw * JITTER_LOW, raw]`: `mul_f64` rounds to the nearest
/// nanosecond, so rounding alone could otherwise take a term a nanosecond outside its band.
fn scaled(raw: Duration, factor: f64) -> Duration {
    raw.mul_f64(factor.clamp(JITTER_LOW, 1.0))
        .min(raw)
        .max(raw.mul_f64(JITTER_LOW))
}

/// SplitMix64 avalanche (Steele et al. 2014) of one 64-bit value: three
/// shift-xor-multiply rounds, returning a non-zero state for every input.
fn avalanche(value: u64) -> u64 {
    let mut z = value;
    z = (z ^ (z >> SPLITMIX_SHIFT_A)).wrapping_mul(SPLITMIX_MUL_A);
    z = (z ^ (z >> SPLITMIX_SHIFT_B)).wrapping_mul(SPLITMIX_MUL_B);
    (z ^ (z >> SPLITMIX_SHIFT_C)) | NON_ZERO_STATE_BIT
}

/// The bit forced into a seeded xorshift state so it is never all zero.
const NON_ZERO_STATE_BIT: u64 = 1;

/// SplitMix64 constants (Steele et al. 2014).
const SPLITMIX_MUL_A: u64 = 0xBF58_476D_1CE4_E5B9;
const SPLITMIX_MUL_B: u64 = 0x94D0_49BB_1331_11EB;
const SPLITMIX_SHIFT_A: u32 = 30;
const SPLITMIX_SHIFT_B: u32 = 27;
const SPLITMIX_SHIFT_C: u32 = 31;

/// Mixes the caller's own entropy into a seed for [`RetrySchedule::new`].
///
/// The caller supplies no clock: it passes whatever it has (a nanosecond
/// reading, a stream id, a route index) and this avalanches it (SplitMix64,
/// Steele et al. 2014) so that adjacent inputs give unrelated seeds. It is a
/// pure function, so the same entropy always yields the same seed — a caller
/// that needs two processes to differ supplies entropy that differs, which is
/// why this crate reads no counter or clock of its own.
#[must_use]
pub fn mix_seed(entropy: u64) -> u64 {
    avalanche(entropy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeSet;
    use alloc::vec::Vec;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn sequence(s: &RetrySchedule, attempts: u32) -> Vec<Duration> {
        (0..attempts).map(|i| s.delay_for_attempt(i)).collect()
    }

    #[test]
    fn the_raw_series_doubles_from_the_floor_and_saturates_at_the_cap() {
        let s = RetrySchedule::new(ms(500), secs(30), 2.0, 1);
        assert_eq!(s.raw_delay_for_attempt(0), ms(500));
        assert_eq!(s.raw_delay_for_attempt(1), ms(1_000));
        assert_eq!(s.raw_delay_for_attempt(2), ms(2_000));
        assert_eq!(s.raw_delay_for_attempt(3), ms(4_000));
        assert_eq!(s.raw_delay_for_attempt(6), secs(30));
        assert_eq!(s.raw_delay_for_attempt(7), secs(30));
    }

    #[test]
    fn an_attempt_past_the_walk_bound_is_the_cap_without_iterating() {
        let s = RetrySchedule::new(secs(1), secs(30), 2.0, 1);
        assert_eq!(s.raw_delay_for_attempt(u32::MAX), secs(30));
        assert_eq!(s.raw_delay_for_attempt(MAX_ATTEMPT_INDEX + 1), secs(30));
    }

    #[test]
    fn the_first_retry_is_exactly_the_configured_step() {
        for (min, cap, factor) in [
            (ms(500), secs(30), 2.0),
            (ms(2_000), ms(10_000), 2.0),
            (secs(1), secs(1), 1.0),
        ] {
            let s = RetrySchedule::new(min, cap, factor, 0xDEAD_BEEF);
            assert_eq!(s.delay_for_attempt(0), s.raw_delay_for_attempt(0));
        }
    }

    #[test]
    fn every_later_delay_stays_in_the_configured_band() {
        let s = RetrySchedule::new(ms(500), secs(30), 2.0, 0xDEAD_BEEF);
        for attempt in 0..64 {
            let raw = s.raw_delay_for_attempt(attempt);
            let d = s.delay_for_attempt(attempt);
            assert!(
                d >= raw.mul_f64(JITTER_LOW) && d <= raw,
                "attempt {attempt}: {d:?} of {raw:?}"
            );
        }
    }

    #[test]
    fn the_delay_stays_jittered_at_the_cap() {
        let s = RetrySchedule::new(ms(500), secs(30), 2.0, 7);
        let mut distinct = BTreeSet::new();
        for _ in 0..16 {
            let d = s.delay_for_attempt(20);
            assert!(d >= secs(25) && d <= secs(30), "{d:?}");
            distinct.insert(d.as_nanos());
        }
        assert!(distinct.len() > 1, "a capped delay must not be constant");
    }

    #[test]
    fn the_same_seed_reproduces_the_same_sequence() {
        let a = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED);
        let b = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED);
        assert_eq!(sequence(&a, 8), sequence(&b, 8));
    }

    #[test]
    fn a_different_seed_draws_differently() {
        let a = RetrySchedule::new(secs(10), secs(30), 2.0, 1);
        let b = RetrySchedule::new(secs(10), secs(30), 2.0, 2);
        assert_ne!(sequence(&a, 8), sequence(&b, 8));
    }

    #[test]
    fn a_clone_reseeds_rather_than_replaying() {
        let a = RetrySchedule::new(ms(500), secs(30), 2.0, 3);
        let b = a.clone();
        assert_ne!(sequence(&a, 8), sequence(&b, 8));
    }

    #[test]
    fn an_offset_starts_the_sequence_elsewhere_in_the_same_band() {
        let base = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED);
        let raw = base.raw_delay_for_attempt(1);
        let mut distinct = BTreeSet::new();
        for offset in 0..JITTER_TERMS {
            let d = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED)
                .with_offset(offset)
                .delay_for_attempt(1);
            assert!(d >= raw.mul_f64(JITTER_LOW) && d <= raw, "{offset}: {d:?}");
            distinct.insert(d.as_nanos());
        }
        assert!(distinct.len() > 1, "each offset must reach its own term");
        let wrapped = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED).with_offset(JITTER_TERMS);
        let first = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED).with_offset(0);
        assert_eq!(wrapped.delay_for_attempt(1), first.delay_for_attempt(1));
    }

    #[test]
    fn an_attempt_is_the_same_term_however_often_it_is_asked() {
        let s = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED);
        let seq = sequence(&s, 4);
        assert_eq!(seq[0], s.raw_delay_for_attempt(0));
        for (attempt, d) in seq.iter().enumerate().skip(1) {
            assert!(
                *d >= s.raw_delay_for_attempt(attempt as u32).mul_f64(JITTER_LOW),
                "{attempt}: {d:?}"
            );
        }
    }

    #[test]
    fn mixed_seeds_are_pure_and_adjacent_entropy_does_not_share_a_sequence() {
        assert_eq!(mix_seed(7), mix_seed(7), "one entropy value, one seed");
        assert_ne!(mix_seed(7), mix_seed(8));
        for entropy in [0, 1, 2, u64::MAX, 41] {
            assert_ne!(avalanche(entropy), 0, "a xorshift state is never zero");
        }
        let a = RetrySchedule::new(ms(500), secs(30), 2.0, mix_seed(1));
        let b = RetrySchedule::new(ms(500), secs(30), 2.0, mix_seed(2));
        assert_ne!(sequence(&a, 8), sequence(&b, 8));
    }

    #[test]
    fn a_factor_below_one_does_not_shrink_the_series() {
        let s = RetrySchedule::new(ms(500), secs(30), 0.25, 1);
        assert_eq!(s.raw_delay_for_attempt(0), ms(500));
        assert_eq!(s.raw_delay_for_attempt(5), ms(500));
    }

    #[test]
    fn a_cap_below_the_floor_is_raised_to_the_floor() {
        let s = RetrySchedule::new(secs(30), secs(1), 2.0, 1);
        assert_eq!(s.max(), secs(30));
        assert_eq!(s.raw_delay_for_attempt(9), secs(30));
    }

    #[test]
    fn a_zero_floor_is_a_zero_raw_delay_and_a_zero_band() {
        let s = RetrySchedule::new(Duration::ZERO, secs(1), 2.0, 1);
        assert_eq!(s.raw_delay_for_attempt(0), Duration::ZERO);
        assert_eq!(s.delay_for_attempt(0), Duration::ZERO);
        assert_eq!(s.delay_for_attempt(1), Duration::ZERO);
    }
}
