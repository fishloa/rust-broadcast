//! One capped, jittered retry schedule for every caller that retries on a
//! timer (issue #2471: the workspace carried five-plus copies).
//!
//! A schedule is pure policy: it answers "how long before attempt `n`?" and
//! never sleeps, reads a clock, or allocates. The caller owns the timer
//! (`tokio::time::sleep`, a TIMERFD, a driver's own wake-up) and the clock
//! (`Instant::now`), which is what keeps every consumer of this module
//! clock-free and testable against a fixed seed. It is `no_std` and uses
//! nothing but integer arithmetic and `f64` multiplication; the workspace's
//! `thumbv7em-none-eabi` build needs no float library for it.
//!
//! # The series
//!
//! A schedule is either exponential (`min * factor^attempt`, capped at `max`)
//! or a literal step table ([`RetrySchedule::from_steps`]).
//!
//! # Jitter
//!
//! Every delay but the first is the series' value times a term of
//! `[5/6, 1.0)`, so it never exceeds the configured step and never falls more
//! than a sixth below it. The first retry is exactly its step: that is the
//! most visible statement a caller makes about how quickly it tries again, and
//! a schedule must not halve it on a draw.
//!
//! Jitter is applied *after* the cap deliberately: a jitter added on top and
//! then re-clamped to `max` (the shape `backon`'s `with_jitter` produces)
//! yields exactly `max` at the cap, i.e. no spread at all — and the cap is
//! where a long outage lives, so a fleet of clients pointed at one dead server
//! must not lockstep there.
//!
//! The terms of one schedule form a fixed sequence drawn from the caller's
//! seed ([`mix_seed`] avalanches whatever entropy the caller has into one). A
//! client's attempts walk that sequence rather than re-rolling it, and
//! [`RetrySchedule::with_offset`] starts a client further into it, which is how
//! a fleet spread over the retry interval stays spread.
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
//! ).expect("a cap above the floor, a factor of two");
//! assert_eq!(schedule.delay_for_attempt(0), Duration::from_millis(500));
//! for attempt in 1..8 {
//!     let delay = schedule.delay_for_attempt(attempt);
//!     assert!(delay >= schedule.raw_delay_for_attempt(attempt) * 5 / 6);
//!     assert!(delay <= schedule.raw_delay_for_attempt(attempt));
//! }
//! ```

use core::cell::Cell;
use core::fmt;
use core::time::Duration;

/// The highest attempt the exponential series is raised to; well before it
/// the raw delay has saturated at the cap for any sane factor, so every later
/// attempt returns the cap without iterating.
const MAX_ATTEMPT_INDEX: u32 = 30;

/// The attempt index of the first retry (and of the sequence's first term).
const FIRST_RETRY: u32 = 0;

/// The sequence's first term, exactly one: the first retry is the raw delay.
const FIRST_TERM: usize = FIRST_RETRY as usize;

/// The jitter band's width as a fraction of the series value. A sixth rather
/// than a half because the jitter exists to break lockstep, not to shorten a
/// schedule.
const JITTER_SPREAD: f64 = 1.0 / 6.0;

/// The lower edge of a jittered term: `raw * 5/6`.
const JITTER_LOW: f64 = 1.0 - JITTER_SPREAD;

/// The upper edge of a jittered term's factor: exactly the raw delay.
const JITTER_TOP: f64 = 1.0;

/// How many jittered terms a schedule's sequence holds: enough for a fleet
/// spread over the retry interval to map onto distinct terms.
const JITTER_TERMS: usize = 16;

/// A growth factor that leaves the series at its floor.
const NO_GROWTH: f64 = 1.0;

/// The largest growth [`term_power`] is taken past its floor; a larger one is
/// the cap without a product.
const MAX_SCALING_FACTOR: f64 = 1e15;

/// The largest floor a schedule may be built with: the largest whole
/// `Duration` second. A visit is computed in `f64`, which holds any whole
/// second exactly, so a floor at or below this one keeps every visit it can
/// reach inside `Duration`; [`Duration::MAX`]'s own nanosecond part is a hair
/// above the largest `f64` second.
pub const MAX_FLOOR: Duration = Duration::new(MAX_FLOOR_SECS, 0);

const MAX_FLOOR_SECS: u64 = u64::MAX / NANOS_PER_SEC;

const NANOS_PER_SEC: u64 = 1_000_000_000;

/// Why [`RetrySchedule::new`] refused its bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum InvalidBackoff {
    /// The cap is under the floor: no attempt may wait longer than the cap and
    /// the first waits exactly the floor.
    CapBelowFloor {
        /// The first retry's delay.
        floor: Duration,
        /// The requested cap.
        cap: Duration,
    },
    /// The floor is above [`MAX_FLOOR`], where the series' own visits overflow
    /// their representation.
    FloorAboveCeiling {
        /// The first retry's delay (or a step table's cap).
        floor: Duration,
        /// [`MAX_FLOOR`], the largest accepted.
        ceiling: Duration,
    },
    /// The growth factor is below one or NaN, so the series would shrink or
    /// re-roll into a shorter one.
    GrowthBelowOne {
        /// The requested factor.
        factor: f64,
    },
}

impl fmt::Display for InvalidBackoff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapBelowFloor { floor, cap } => {
                write!(f, "retry cap {cap:?} is below the first retry's {floor:?}")
            }
            Self::FloorAboveCeiling { floor, ceiling } => write!(
                f,
                "retry floor {floor:?} is above the largest accepted {ceiling:?}"
            ),
            Self::GrowthBelowOne { factor } => {
                write!(f, "retry growth factor {factor} is under 1")
            }
        }
    }
}

impl core::error::Error for InvalidBackoff {}

impl Eq for InvalidBackoff {}

/// xorshift64 shift triple (Marsaglia 2003).
const XORSHIFT_LEFT_A: u32 = 13;
const XORSHIFT_RIGHT_B: u32 = 7;
const XORSHIFT_LEFT_C: u32 = 17;

/// How many high bits of the xorshift state become the unit fraction.
const UNIT_FRACTION_BITS: u32 = 24;
/// `1 << UNIT_FRACTION_BITS`, the divisor turning those bits into `[0, 1)`.
const UNIT_FRACTION_SCALE: u32 = 1 << UNIT_FRACTION_BITS;

/// SplitMix64 constants (Steele et al. 2014).
const SPLITMIX_MUL_A: u64 = 0xBF58_476D_1CE4_E5B9;
const SPLITMIX_MUL_B: u64 = 0x94D0_49BB_1331_11EB;
const SPLITMIX_SHIFT_A: u32 = 30;
const SPLITMIX_SHIFT_B: u32 = 27;
const SPLITMIX_SHIFT_C: u32 = 31;

/// The bit forced into a seeded xorshift state so it is never all zero.
const NON_ZERO_STATE_BIT: u64 = 1;

/// A capped, jittered retry schedule.
///
/// [`delay_for_attempt`](Self::delay_for_attempt) is the delay a caller
/// waits; [`raw_delay_for_attempt`](Self::raw_delay_for_attempt) is the
/// deterministic series it is drawn from.
///
/// # Concurrency
///
/// The position in the jitter sequence is a `Cell`, so a schedule is `Send` but
/// not `Sync`, and `delay_for_attempt` takes `&self` while advancing it. One
/// schedule belongs to one task. [`Clone`] reseeds rather than duplicating the
/// state, so a clone never replays its original's sequence.
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
    Steps(alloc::vec::Vec<Duration>),
}

impl RetrySchedule {
    /// A schedule from `min` (the first raw delay), `max` (the cap), a growth
    /// factor and a jitter seed.
    ///
    /// # Errors
    ///
    /// [`InvalidBackoff::CapBelowFloor`] for a `max` under `min`,
    /// [`InvalidBackoff::FloorAboveCeiling`] for a `min` above
    /// [`MAX_FLOOR`], and [`InvalidBackoff::GrowthBelowOne`] for a `factor`
    /// under one or NaN (the series could shrink, or re-roll into a shorter
    /// one). None is repaired into a different schedule.
    pub fn new(
        min: Duration,
        max: Duration,
        factor: f64,
        seed: u64,
    ) -> Result<Self, InvalidBackoff> {
        if min > MAX_FLOOR {
            return Err(InvalidBackoff::FloorAboveCeiling {
                floor: min,
                ceiling: MAX_FLOOR,
            });
        }
        if max < min {
            return Err(InvalidBackoff::CapBelowFloor {
                floor: min,
                cap: max,
            });
        }
        if factor.is_nan() || factor < NO_GROWTH {
            return Err(InvalidBackoff::GrowthBelowOne { factor });
        }
        Ok(Self {
            kind: Kind::Exponential { min, factor },
            max,
            terms: jitter_terms(seed),
            cursor: Cell::new(FIRST_TERM + 1),
        })
    }

    /// A schedule from `min`, `max` and a growth factor, seeded from
    /// `entropy` through [`mix_seed`].
    ///
    /// # Errors
    ///
    /// Whatever [`Self::new`] rejects.
    pub fn seeded(
        min: Duration,
        max: Duration,
        factor: f64,
        entropy: u64,
    ) -> Result<Self, InvalidBackoff> {
        Self::new(min, max, factor, mix_seed(entropy))
    }

    /// A schedule whose series is the literal table `steps` (the first entry is
    /// the first retry's delay), then `cap` for every later attempt.
    ///
    /// This is the shape a caller with a hand-chosen step table has, kept
    /// distinct from [`Self::new`] so an existing table becomes configuration of
    /// one schedule rather than being re-derived as an exponential series that
    /// only happens to pass through the same numbers. The table is kept as
    /// given (a step may exceed `cap`, which bounds only the exponential
    /// series); an empty table is the cap from the first attempt.
    ///
    /// # Errors
    ///
    /// Whatever [`Self::new`] rejects for the cap and ceiling; the table
    /// itself is not bounded.
    pub fn from_steps(
        steps: &[Duration],
        cap: Duration,
        seed: u64,
    ) -> Result<Self, InvalidBackoff> {
        if cap > MAX_FLOOR {
            return Err(InvalidBackoff::FloorAboveCeiling {
                floor: cap,
                ceiling: MAX_FLOOR,
            });
        }
        Ok(Self {
            kind: Kind::Steps(steps.to_vec()),
            max: cap,
            terms: jitter_terms(seed),
            cursor: Cell::new(FIRST_TERM + 1),
        })
    }

    /// The same schedule with the retry that has been cancelled starting at
    /// `offset` of its jitter sequence instead of at the top.
    ///
    /// A fleet spread over the retry interval can map each client to its own
    /// offset, so the retry band keeps the spread of the instant it replaced
    /// rather than collapsing onto one term. Offset `0` is the schedule as
    /// built (the first retry is exactly the raw delay).
    #[must_use]
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.cursor = Cell::new(offset % JITTER_TERMS);
        self
    }

    /// The delay before the `attempt + 1`-th retry: the raw delay itself for
    /// the first, then the raw delay times the next term of the schedule's
    /// jitter sequence (wrapping at the end of the sequence), in `[raw*5/6, raw]`.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == FIRST_RETRY {
            return self.raw_delay_for_attempt(attempt);
        }
        let cursor = self.cursor.get() % JITTER_TERMS;
        self.cursor.set((cursor + 1) % JITTER_TERMS);
        self.jittered(attempt, cursor)
    }

    /// The delay before the `attempt + 1`-th retry for the jitter term at
    /// `cursor`, without advancing the schedule.
    #[must_use]
    pub fn peek_delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == FIRST_RETRY {
            return self.raw_delay_for_attempt(attempt);
        }
        self.jittered(attempt, self.cursor.get() % JITTER_TERMS)
    }

    fn jittered(&self, attempt: u32, cursor: usize) -> Duration {
        let raw = self.raw_delay_for_attempt(attempt);
        let capped = attempt.min(MAX_ATTEMPT_INDEX);
        scaled(raw, term_power(self.terms[cursor], capped), self.max)
    }

    /// The delay before the `attempt + 1`-th retry without jitter:
    /// `min * factor^attempt` capped at `max`, or the literal step table's
    /// entry.
    #[must_use]
    pub fn raw_delay_for_attempt(&self, attempt: u32) -> Duration {
        match &self.kind {
            Kind::Steps(steps) => steps
                .get(usize::try_from(attempt).unwrap_or(usize::MAX))
                .copied()
                .unwrap_or(self.max),
            Kind::Exponential { min, factor } => {
                if attempt > MAX_ATTEMPT_INDEX {
                    return self.max;
                }
                // A zero floor is a zero delay at every attempt: it is how a
                // caller disables backoff (a round-trip delay, never a burst,
                // because the caller still awaits the retry itself).
                if min.is_zero() {
                    return Duration::ZERO;
                }
                let grown = term_power(*factor, attempt);
                if !grown.is_finite() || grown > MAX_SCALING_FACTOR {
                    return self.max;
                }
                // `mul_f64` panics on a product `Duration` cannot hold; one that
                // would is the cap, which this schedule's bounds keep reachable.
                let visit = in_bounds(*min, grown, self.max);
                if visit >= self.max { self.max } else { visit }
            }
        }
    }

    /// The first (unjittered) delay of the series.
    #[must_use]
    pub fn min(&self) -> Duration {
        match &self.kind {
            Kind::Steps(steps) => steps.first().copied().unwrap_or(self.max),
            Kind::Exponential { min, .. } => *min,
        }
    }

    /// The cap the series saturates at.
    #[must_use]
    pub fn max(&self) -> Duration {
        self.max
    }
}

/// A retry step table carried as configuration, not as runtime state.
///
/// [`RetrySchedule`] holds a `Cell` cursor and reseeds on [`Clone`], so it is
/// not a faithful config value; `BackoffSteps` is the plain, `Clone`-faithful
/// pair every config struct holds. It answers nothing on its own — call
/// [`BackoffSteps::schedule`] to turn it into a [`RetrySchedule`] for a given
/// jitter seed.
#[derive(Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BackoffSteps {
    /// Per-attempt delays; the first entry is the first retry's delay. An empty
    /// table is the cap from the first attempt.
    pub steps: alloc::vec::Vec<Duration>,
    /// The delay every attempt past the table takes.
    pub cap: Duration,
}

impl BackoffSteps {
    /// Build a [`RetrySchedule`] from these steps with the caller's own jitter
    /// `seed` (see [`RetrySchedule::from_steps`]).
    ///
    /// # Errors
    ///
    /// Whatever [`RetrySchedule::from_steps`] rejects.
    pub fn schedule(&self, seed: u64) -> Result<RetrySchedule, InvalidBackoff> {
        RetrySchedule::from_steps(&self.steps, self.cap, seed)
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

/// `base` raised to `exponent`, multiplied out one factor at a time: `core`
/// has no `powi`, and `exponent` is bounded by [`MAX_ATTEMPT_INDEX`].
fn term_power(base: f64, exponent: u32) -> f64 {
    let mut power = 1.0;
    for _ in 0..exponent {
        power *= base;
    }
    power
}

/// `raw * factor`, computed only when [`Duration`] can hold the product: a
/// product whose whole-second ceiling is at or above [`MAX_FLOOR`] is `cap`
/// without the multiplication. A schedule's cap never exceeds [`MAX_FLOOR`], so
/// `cap` is what the product would have been clamped to anyway.
fn in_bounds(raw: Duration, factor: f64, cap: Duration) -> Duration {
    let ceiling = (raw.as_secs_f64() * factor).floor();
    if !ceiling.is_finite() || ceiling >= MAX_FLOOR_SECS as f64 {
        cap
    } else {
        raw.mul_f64(factor)
    }
}

/// `raw * factor`, kept inside `[raw * JITTER_LOW, raw]`: `mul_f64` rounds to
/// the nearest nanosecond, so rounding alone could take a term just outside its
/// band, and a factor outside the band (only reachable through an absurd seed
/// sample) is the raw delay itself. Band edges beyond [`MAX_FLOOR`] are the raw
/// delay, which the schedule's floor already bounds.
fn scaled(raw: Duration, factor: f64, cap: Duration) -> Duration {
    if !factor.is_finite() || factor >= JITTER_TOP {
        return raw;
    }
    let finite = |visit: Duration| visit <= cap;
    let product = in_bounds(raw, factor.max(JITTER_LOW), cap);
    let low = in_bounds(raw, JITTER_LOW, cap);
    if finite(product) && finite(low) {
        product.min(raw).max(low)
    } else {
        raw
    }
}

/// The schedule's jitter sequence: [`JITTER_TERMS`] terms of `[5/6, 1.0)`, the
/// first of which is exactly one.
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

/// SplitMix64 avalanche (Steele et al. 2014) of one 64-bit value: three
/// shift-xor-multiply rounds, returning a non-zero state for every input.
fn avalanche(value: u64) -> u64 {
    let mut z = value;
    z = (z ^ (z >> SPLITMIX_SHIFT_A)).wrapping_mul(SPLITMIX_MUL_A);
    z = (z ^ (z >> SPLITMIX_SHIFT_B)).wrapping_mul(SPLITMIX_MUL_B);
    (z ^ (z >> SPLITMIX_SHIFT_C)) | NON_ZERO_STATE_BIT
}

/// Mixes the caller's own entropy into a seed for [`RetrySchedule::new`].
///
/// The caller supplies no clock: it passes whatever it has (a nanosecond
/// reading, a stream id, a route index) and this avalanches it, so adjacent
/// inputs give unrelated sequences. It is a pure function, so the same entropy
/// always yields the same seed: a caller that needs two processes to differ
/// supplies entropy that differs, which is why this crate reads no counter or
/// clock of its own.
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

    fn sequence(terms: usize) -> Vec<Duration> {
        let s = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED).unwrap();
        let mut out = Vec::with_capacity(terms);
        for attempt in 0..terms as u32 {
            out.push(s.delay_for_attempt(attempt));
        }
        out
    }

    #[test]
    fn the_raw_series_doubles_from_the_floor_and_saturates_at_the_cap() {
        let s = RetrySchedule::new(ms(500), secs(30), 2.0, 1).unwrap();
        assert_eq!(s.raw_delay_for_attempt(0), ms(500));
        assert_eq!(s.raw_delay_for_attempt(1), ms(1_000));
        assert_eq!(s.raw_delay_for_attempt(2), ms(2_000));
        assert_eq!(s.raw_delay_for_attempt(3), ms(4_000));
        assert_eq!(s.raw_delay_for_attempt(6), secs(30));
        assert_eq!(s.raw_delay_for_attempt(7), secs(30));
    }

    #[test]
    fn an_attempt_past_the_walk_bound_is_the_cap_without_iterating() {
        let s = RetrySchedule::new(secs(1), secs(30), 2.0, 1).unwrap();
        assert_eq!(s.raw_delay_for_attempt(u32::MAX), secs(30));
        assert_eq!(s.raw_delay_for_attempt(MAX_ATTEMPT_INDEX + 1), secs(30));
        assert!(s.delay_for_attempt(u32::MAX) <= secs(30));
    }

    #[test]
    fn the_first_retry_is_exactly_the_configured_step() {
        for (min, cap, factor) in [
            (ms(500), secs(30), 2.0),
            (ms(2_000), ms(10_000), 2.0),
            (secs(1), secs(1), 1.0),
        ] {
            let s = RetrySchedule::new(min, cap, factor, 0xDEAD_BEEF).unwrap();
            assert_eq!(s.delay_for_attempt(0), min);
        }
    }

    #[test]
    fn every_later_delay_stays_in_the_configured_band() {
        let s = RetrySchedule::new(ms(500), secs(30), 2.0, 0xDEAD_BEEF).unwrap();
        for attempt in 0..64 {
            let raw = s.raw_delay_for_attempt(attempt);
            let d = s.delay_for_attempt(attempt);
            let band = raw;
            assert!(
                d >= band.mul_f64(JITTER_LOW) && d <= band,
                "attempt {attempt}: {d:?} of {band:?}"
            );
        }
    }

    #[test]
    fn the_delay_stays_jittered_at_the_cap() {
        let s = RetrySchedule::new(ms(500), secs(30), 2.0, 7)
            .unwrap()
            .with_offset(1);
        let mut distinct = BTreeSet::new();
        for _ in 0..64 {
            let d = s.delay_for_attempt(20);
            assert!(d >= secs(25) && d <= secs(30), "{d:?}");
            distinct.insert(d.as_nanos());
        }
        assert!(distinct.len() > 1, "a capped delay must not be constant");
    }

    #[test]
    fn the_same_seed_reproduces_the_same_sequence() {
        let a = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED).unwrap();
        let b = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED).unwrap();
        let seq = |s: &RetrySchedule| -> Vec<Duration> {
            (0..8).map(|i| s.delay_for_attempt(i)).collect()
        };
        assert_eq!(seq(&a), seq(&b));
    }

    #[test]
    fn a_different_seed_draws_differently() {
        let a = RetrySchedule::new(secs(10), secs(30), 2.0, 1).unwrap();
        let b = RetrySchedule::new(secs(10), secs(30), 2.0, 2).unwrap();
        let seq = |s: &RetrySchedule| -> Vec<Duration> {
            (0..8).map(|i| s.delay_for_attempt(i)).collect()
        };
        assert_ne!(seq(&a), seq(&b));
    }

    #[test]
    fn a_clone_reseeds_rather_than_replaying() {
        let a = RetrySchedule::new(ms(500), secs(30), 2.0, 3).unwrap();
        let b = a.clone();
        let seq = |s: &RetrySchedule| -> Vec<Duration> {
            (0..8).map(|i| s.delay_for_attempt(i)).collect()
        };
        assert_ne!(seq(&a), seq(&b));
    }

    #[test]
    fn an_offset_starts_the_sequence_elsewhere_in_the_same_band() {
        let raw = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED)
            .unwrap()
            .raw_delay_for_attempt(1);
        let mut distinct = BTreeSet::new();
        for offset in 0..JITTER_TERMS {
            let d = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED)
                .unwrap()
                .with_offset(offset)
                .delay_for_attempt(1);
            assert!(d >= raw.mul_f64(JITTER_LOW) && d <= raw, "{offset}: {d:?}");
            distinct.insert(d.as_nanos());
        }
        assert!(distinct.len() > 1, "each offset must reach its own term");
        let wrapped = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED)
            .unwrap()
            .with_offset(JITTER_TERMS)
            .delay_for_attempt(1);
        let first = RetrySchedule::new(ms(500), secs(30), 2.0, 0x5EED)
            .unwrap()
            .with_offset(0)
            .delay_for_attempt(1);
        assert_eq!(wrapped, first);
    }

    #[test]
    fn a_literal_step_table_is_taken_as_given_then_the_cap() {
        let s = RetrySchedule::from_steps(&[ms(2_000), ms(4_000)], ms(10_000), 0x5EED).unwrap();
        assert_eq!(s.raw_delay_for_attempt(0), ms(2_000));
        assert_eq!(s.raw_delay_for_attempt(1), ms(4_000));
        assert_eq!(s.raw_delay_for_attempt(2), ms(10_000));
        assert_eq!(s.raw_delay_for_attempt(u32::MAX), ms(10_000));
        assert_eq!(s.delay_for_attempt(0), ms(2_000), "the first step exactly");
        assert_eq!(
            RetrySchedule::from_steps(&[], ms(7), 1).unwrap().min(),
            ms(7)
        );
    }

    #[test]
    fn mixed_seeds_are_pure_and_adjacent_entropy_does_not_share_a_sequence() {
        assert_eq!(mix_seed(7), mix_seed(7), "one entropy value, one seed");
        assert_ne!(mix_seed(7), mix_seed(8));
        for entropy in [0, 1, 2, u64::MAX, 41] {
            assert_ne!(avalanche(entropy), 0, "a xorshift state is never zero");
        }
        let a = RetrySchedule::seeded(ms(500), secs(30), 2.0, 1).unwrap();
        let b = RetrySchedule::seeded(ms(500), secs(30), 2.0, 2).unwrap();
        let seq = |s: &RetrySchedule| -> Vec<Duration> {
            (0..8).map(|i| s.delay_for_attempt(i)).collect()
        };
        assert_ne!(seq(&a), seq(&b));
    }

    #[test]
    fn a_hostile_factor_saturates_at_the_cap_instead_of_panicking() {
        for factor in [f64::INFINITY, 1e300] {
            let s = RetrySchedule::new(secs(1), secs(60), factor, 1).unwrap();
            for attempt in [0u32, 1, 30, u32::MAX] {
                assert!(
                    s.delay_for_attempt(attempt) <= secs(60),
                    "{factor}: {attempt}"
                );
            }
        }
    }

    #[test]
    fn a_cap_below_the_floor_is_rejected_not_raised() {
        assert_eq!(
            RetrySchedule::new(secs(30), secs(1), 2.0, 1).err(),
            Some(InvalidBackoff::CapBelowFloor {
                floor: secs(30),
                cap: secs(1)
            })
        );
    }

    #[test]
    fn a_floor_above_the_ceiling_is_rejected() {
        assert_eq!(
            RetrySchedule::new(MAX_FLOOR.mul_f64(2.0), secs(30), 2.0, 1).err(),
            Some(InvalidBackoff::FloorAboveCeiling {
                floor: MAX_FLOOR.mul_f64(2.0),
                ceiling: MAX_FLOOR
            })
        );
        let schedule = RetrySchedule::new(MAX_FLOOR, MAX_FLOOR, 2.0, 1).unwrap();
        assert_eq!(schedule.raw_delay_for_attempt(0), MAX_FLOOR);
    }

    #[test]
    fn a_largest_floor_is_exactly_the_first_visit_and_the_cap_beyond_it() {
        let schedule = RetrySchedule::new(MAX_FLOOR, MAX_FLOOR, 2.0, 1).unwrap();
        let first = MAX_FLOOR.mul_f64(JITTER_LOW);
        assert_eq!(schedule.raw_delay_for_attempt(0), MAX_FLOOR);
        assert_eq!(schedule.raw_delay_for_attempt(1), MAX_FLOOR);
        assert_eq!(schedule.raw_delay_for_attempt(40), MAX_FLOOR);
        assert_eq!(schedule.raw_delay_for_attempt(u32::MAX), MAX_FLOOR);
        assert_eq!(schedule.delay_for_attempt(0), MAX_FLOOR);
        assert!(
            schedule.delay_for_attempt(1) >= first && schedule.delay_for_attempt(1) <= MAX_FLOOR
        );
        assert!(schedule.delay_for_attempt(u32::MAX) >= first);
    }

    #[test]
    fn a_factor_below_one_is_rejected_not_raised() {
        for (floor, cap, factor) in [
            (ms(500), secs(30), 0.25),
            (ms(500), secs(30), 0.999),
            (secs(30), secs(30), 0.25),
        ] {
            assert_eq!(
                RetrySchedule::new(floor, cap, factor, 1).err(),
                Some(InvalidBackoff::GrowthBelowOne { factor }),
                "{floor:?} {cap:?} {factor}"
            );
        }
        assert!(matches!(
            RetrySchedule::new(ms(500), secs(30), f64::NAN, 1),
            Err(InvalidBackoff::GrowthBelowOne { factor }) if factor.is_nan()
        ));
    }

    #[test]
    fn a_zero_floor_is_a_zero_delay_at_every_attempt() {
        let s = RetrySchedule::new(Duration::ZERO, secs(1), 2.0, 1).unwrap();
        assert_eq!(s.raw_delay_for_attempt(0), Duration::ZERO);
        assert_eq!(s.delay_for_attempt(0), Duration::ZERO);
        assert_eq!(s.delay_for_attempt(1), Duration::ZERO);
        assert_eq!(
            s.delay_for_attempt(20),
            Duration::ZERO,
            "a disabled backoff never creeps up to its cap"
        );
    }

    #[test]
    fn the_jittered_series_is_finite_for_every_attempt() {
        let seq = sequence(8);
        assert_eq!(seq[0], ms(500));
        assert!(seq.iter().all(|d| *d <= secs(30)));
    }

    #[test]
    fn backoff_steps_is_a_faithful_config_value() {
        let steps = BackoffSteps {
            steps: alloc::vec![ms(2_000), ms(4_000)],
            cap: ms(10_000),
        };
        // A config value clones exactly and compares by value; the runtime schedule it builds
        // carries a cursor cell and reseeds on clone, which is why it cannot be a config field.
        assert_eq!(steps.clone(), steps);
        let schedule = steps.schedule(0x5EED).unwrap();
        assert_eq!(schedule.min(), ms(2_000));
        assert_eq!(schedule.max(), ms(10_000));
        assert_eq!(schedule.raw_delay_for_attempt(0), ms(2_000));
        assert_eq!(schedule.raw_delay_for_attempt(1), ms(4_000));
        assert_eq!(schedule.raw_delay_for_attempt(2), ms(10_000));
        // The seed is the caller's own, so the same steps and seed replay one series.
        let a = steps.schedule(7).unwrap();
        let b = steps.schedule(7).unwrap();
        assert_eq!(a.delay_for_attempt(0), b.delay_for_attempt(0));
        assert_eq!(a.delay_for_attempt(1), b.delay_for_attempt(1));
    }

    #[test]
    fn an_empty_backoff_steps_table_is_the_cap_from_the_first_attempt() {
        let steps = BackoffSteps {
            steps: alloc::vec::Vec::new(),
            cap: ms(7),
        };
        let schedule = steps.schedule(1).unwrap();
        assert_eq!(schedule.raw_delay_for_attempt(0), ms(7));
        assert_eq!(schedule.min(), ms(7));
    }
}
