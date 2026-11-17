//! The retry schedule's jitter sequence, its cursor rotation and the series'
//! bound edges (audit 21, #2920). A degraded PRNG, an off-by-one in the walk or
//! a peek that advances all still land inside `[5/6, 1]`, so a band check alone
//! cannot see them; the delay a reconnecting client waits is pinned exactly
//! here instead.

use core::time::Duration;

use broadcast_common::backoff::{
    GrowthFactor, InvalidBackoff, MAX_FLOOR, ReconnectPolicy, RetrySchedule,
};

/// Terms in one schedule's jitter sequence, pinned here as a literal so a
/// shortened sequence is a failing test rather than a moved expectation.
const TERMS: usize = 16;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn schedule(seed: u64) -> RetrySchedule {
    RetrySchedule::new(secs(1), secs(60), 2.0, seed).expect("a one-second floor under a minute cap")
}

#[test]
fn the_seeded_jitter_sequence_is_pinned_exactly() {
    // One-second floor, factor two, one-minute cap, seed 1. The jitter terms
    // are the xorshift64 / SplitMix64 constants' own output: a changed shift,
    // multiplier or xor chain draws different nanoseconds, which is the whole
    // point (a fleet drawn from one seed must not collapse onto one instant).
    let schedule = schedule(1);
    let delays: Vec<Duration> = (1..=5).map(|a| schedule.delay_for_attempt(a)).collect();
    assert_eq!(
        delays,
        vec![
            Duration::from_nanos(1_742_927_690),
            Duration::from_nanos(3_897_120_277),
            Duration::from_nanos(7_863_557_100),
            Duration::from_nanos(15_763_773_282),
            Duration::from_nanos(27_593_209_585),
        ]
    );
}

#[test]
fn every_seed_draws_its_own_pinned_terms() {
    // Three seeds whose SplitMix64 avalanche differs in both the high and the
    // low bits, so a degraded xor/and/or round or a swapped shift draws a
    // different nanosecond count rather than merely a different spread.
    for (seed, expected) in [
        (1u64, [1_742_927_690u128, 3_897_120_277, 7_863_557_100]),
        (2, [1_709_811_648, 3_629_289_746, 7_920_523_167]),
        (0xDEAD_BEEF, [1_967_735_489, 3_888_299_386, 7_318_741_242]),
    ] {
        let schedule = schedule(seed);
        let delays: Vec<u128> = (1..=3u32)
            .map(|attempt| schedule.delay_for_attempt(attempt).as_nanos())
            .collect();
        assert_eq!(delays, expected.to_vec(), "seed {seed:#x}");
    }
}

#[test]
fn the_jittered_band_is_spread_for_every_seed() {
    for seed in [1u64, 2, 0xDEAD_BEEF] {
        let schedule = schedule(seed);
        let mut distinct = std::collections::BTreeSet::new();
        for attempt in 1..=16u32 {
            let raw = schedule.raw_delay_for_attempt(attempt);
            let delay = schedule.delay_for_attempt(attempt);
            assert!(
                delay >= raw.mul_f64(5.0 / 6.0) && delay <= raw,
                "seed {seed} attempt {attempt}: {delay:?} of {raw:?}"
            );
            distinct.insert(delay.as_nanos());
        }
        assert!(
            distinct.len() >= 12,
            "seed {seed} repeats a term: {} distinct of 16",
            distinct.len()
        );
    }
    let first = schedule(1);
    let second = schedule(2);
    let take = |s: &RetrySchedule| -> Vec<Duration> {
        (1..=16u32).map(|a| s.delay_for_attempt(a)).collect()
    };
    assert_ne!(take(&first), take(&second));
}

#[test]
fn the_sequence_wraps_after_the_last_term() {
    let schedule = schedule(1);
    let calls: Vec<Duration> = (0..TERMS + 3)
        .map(|_| schedule.delay_for_attempt(1))
        .collect();
    for k in 0..3 {
        assert_eq!(calls[k], calls[k + TERMS], "call {k} wraps");
    }
}

#[test]
fn an_offset_starts_the_walk_at_that_term() {
    let built = schedule(1);
    let walked: Vec<Duration> = (0..TERMS).map(|_| built.delay_for_attempt(1)).collect();
    for offset in 0..TERMS {
        let started = schedule(1).with_offset(offset).delay_for_attempt(1);
        assert_eq!(
            started,
            walked[(offset + TERMS - 1) % TERMS],
            "offset {offset} starts on the term the walk reaches at that index"
        );
    }
    let wrapped = schedule(1).with_offset(TERMS + 3);
    let plain = schedule(1).with_offset(3);
    assert_eq!(
        wrapped.delay_for_attempt(1),
        plain.delay_for_attempt(1),
        "an offset is taken modulo the sequence length"
    );
    let huge = schedule(1).with_offset(usize::MAX);
    let reduced = schedule(1).with_offset(usize::MAX % TERMS);
    assert_eq!(
        huge.delay_for_attempt(1),
        reduced.delay_for_attempt(1),
        "the largest offset still names a term"
    );
}

#[test]
fn a_peek_reads_the_next_term_without_advancing() {
    let schedule = schedule(1);
    assert_eq!(
        schedule.peek_delay_for_attempt(0),
        secs(1),
        "the first retry is the raw delay, peeked or not"
    );
    assert_eq!(
        schedule.peek_delay_for_attempt(1),
        schedule.peek_delay_for_attempt(1)
    );
    let peeked = schedule.peek_delay_for_attempt(1);
    assert!(peeked > Duration::ZERO, "a peek is never a zero delay");
    assert_eq!(
        schedule.delay_for_attempt(1),
        peeked,
        "a peek answers the delay the next call returns"
    );
    assert_eq!(
        schedule.peek_delay_for_attempt(2),
        schedule.delay_for_attempt(2)
    );
}

#[test]
fn the_series_is_raised_only_to_the_walk_bound() {
    let schedule = RetrySchedule::new(secs(1), MAX_FLOOR, 2.0, 1).expect("a floor under the cap");
    assert_eq!(schedule.raw_delay_for_attempt(29), secs(536_870_912));
    assert_eq!(schedule.raw_delay_for_attempt(30), secs(1_073_741_824));
    assert_eq!(schedule.raw_delay_for_attempt(31), MAX_FLOOR);
    assert_eq!(schedule.raw_delay_for_attempt(u32::MAX), MAX_FLOOR);
}

#[test]
fn a_growth_past_the_scaling_ceiling_is_the_cap() {
    // A one-nanosecond floor with a factor of ten: attempt 15 raises the floor
    // by exactly the largest scaling factor the series multiplies out, and
    // attempt 16 passes it.
    let schedule = RetrySchedule::new(Duration::from_nanos(1), MAX_FLOOR, 10.0, 1)
        .expect("a floor under the cap");
    assert_eq!(schedule.raw_delay_for_attempt(14), secs(100_000));
    assert_eq!(schedule.raw_delay_for_attempt(15), secs(1_000_000));
    assert_eq!(schedule.raw_delay_for_attempt(16), MAX_FLOOR);
    assert_eq!(schedule.raw_delay_for_attempt(17), MAX_FLOOR);
}

#[test]
fn a_step_table_accepts_the_largest_cap_and_refuses_one_above_it() {
    assert!(RetrySchedule::from_steps(&[], MAX_FLOOR, 1).is_ok());
    assert_eq!(
        RetrySchedule::from_steps(&[], MAX_FLOOR + secs(1), 1).err(),
        Some(InvalidBackoff::FloorAboveCeiling {
            floor: MAX_FLOOR + secs(1),
            ceiling: MAX_FLOOR,
        })
    );
}

#[test]
fn a_step_table_is_jittered_past_its_first_entry() {
    let schedule =
        RetrySchedule::from_steps(&[ms(2_000), ms(4_000)], secs(10), 1).expect("a ten-second cap");
    assert_eq!(schedule.raw_delay_for_attempt(1), ms(4_000));
    assert_eq!(
        schedule.delay_for_attempt(1),
        Duration::from_nanos(3_485_855_381)
    );
    // A clone reseeds, but it still holds a jitter cursor: its second attempt
    // is a jittered term, never the bare raw delay.
    let cloned = schedule.clone();
    assert_eq!(cloned.raw_delay_for_attempt(1), ms(4_000));
    assert!(cloned.delay_for_attempt(1) < ms(4_000));
}

#[test]
fn doubling_walks_powers_of_two_up_to_the_cap() {
    let policy = ReconnectPolicy::doubling(secs(1), secs(4));
    assert_eq!(policy.backoff.steps, vec![secs(1), secs(2)]);
    assert_eq!(policy.backoff.cap, secs(4));
    let schedule = policy.backoff.schedule(1).expect("a four-second cap");
    assert_eq!(schedule.raw_delay_for_attempt(0), secs(1));
    assert_eq!(schedule.raw_delay_for_attempt(1), secs(2));
    assert_eq!(schedule.raw_delay_for_attempt(2), secs(4));
    assert_eq!(schedule.raw_delay_for_attempt(9), secs(4));
    assert_eq!(schedule.delay_for_attempt(0), secs(1));

    let raised = ReconnectPolicy::doubling(secs(5), secs(4));
    assert_eq!(raised.backoff.steps, Vec::new());
    assert_eq!(raised.backoff.cap, secs(5));
    assert_eq!(
        raised
            .backoff
            .schedule(1)
            .expect("a five-second cap")
            .raw_delay_for_attempt(0),
        secs(5)
    );

    let from_zero = ReconnectPolicy::doubling(Duration::ZERO, ms(4));
    assert_eq!(from_zero.backoff.steps, vec![ms(1), ms(2)]);
    assert_eq!(from_zero.backoff.cap, ms(4));
}

#[test]
fn the_growth_factor_description_is_its_label() {
    assert_eq!(GrowthFactor::Nan.name(), "is NaN");
    assert_eq!(GrowthFactor::Below(0.5).name(), "is under 1");
    assert_eq!(GrowthFactor::Nan.to_string(), "is NaN");
    assert_eq!(GrowthFactor::Below(0.5).to_string(), "0.5 is under 1");
}
