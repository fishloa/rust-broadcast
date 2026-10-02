//! Generic 33-bit wrapping-clock helpers.
//!
//! ISO/IEC 13818-1 §2.4.3.7 samples a 90 kHz clock into a 33-bit PTS/DTS
//! field; ANSI/SCTE 35 §9.2 `pts_time` reuses the identical 2^33 modulus so a
//! splice cue can be compared against the same clock. Both wrap roughly every
//! 26.5 hours, and any long-lived consumer that needs an ever-growing
//! timeline, or just needs to compare two nearby samples correctly across a
//! wrap boundary, needs the same handful of primitives. Before this module
//! existed, four crates (`timed-metadata`, `transmux`, `media-doctor`,
//! `compliance-probe`) each hand-rolled their own copy; an overrun or
//! wrap-direction fix in one reached none of the others. This module is now
//! the single owner both algorithms live in.
//!
//! `transmux` (a container-muxing hub) cannot take a dependency on
//! `timed-metadata` (a DPI/timed-metadata *signalling* conversion crate several
//! layers up the stack, pulling in `scte35-splice`/`mp4-emsg`) without an
//! inverted, heavy dependency edge, and the primitive itself has no
//! dependencies of its own — so it lives here, in the crate every one of the
//! four already depends on, rather than promoting one sibling to depend on
//! another.
//!
//! Two independent operations live here, because they answer different
//! questions and must not be collapsed into one:
//!
//! - [`unwrap_delta`] — extend a running **unwrapped** (ever-growing, signed)
//!   accumulator by the next raw sample, correcting for exactly one wrap in
//!   *either* direction. Used to turn a repeating hardware counter into an
//!   absolute timeline (PTS/DTS unrolling across a capture, including
//!   B-frame reordering that dips slightly backward without crossing a
//!   wrap).
//! - [`wrapping_forward_distance`] — the modular forward distance from one
//!   already-comparable raw value to another, with no accumulator or history
//!   at all. Used to classify a single pair of values as "in order" vs
//!   "wrapped/out of order" when the caller already knows the two are
//!   supposed to be close in time (e.g. a decode-order monotonicity check,
//!   or a splice cue's `pts_time` judged against a reference "now").

/// The 33-bit modulus (2^33) shared by MPEG-2 Systems PTS/DTS (ISO/IEC
/// 13818-1 §2.4.3.7) and SCTE-35 `pts_time` (ANSI/SCTE 35 §9.2) — both a
/// 90 kHz clock sampled into a 33-bit field.
pub const WRAP_33BIT: u64 = 1 << 33;

/// Half of [`WRAP_33BIT`] — the threshold distinguishing a genuine backward
/// step from a legal wrap.
pub const WRAP_33BIT_HALF: u64 = WRAP_33BIT / 2;

/// Extend a running unwrapped 33-bit clock by the delta to the next raw
/// value, correcting for a single wrap in either direction.
///
/// The delta is computed on the wrapped clock (a signed value in
/// `(-2^32, 2^32]`), then applied to the unwrapped accumulator — so an
/// ordinary small backward step (e.g. B-frame PTS reordering) is preserved
/// as-is, and only a near-full-range jump is treated as a wrap.
/// `prev_unwrapped` need not itself be in `[0, 2^33)`; after the first wrap
/// it grows (or, in a reorder that dips across the origin before any wrap
/// has happened, can go slightly negative) without bound.
///
/// This is deliberately **bidirectional**: a naive "epoch counter that only
/// ever increments" unroller (which is what this replaced in
/// `timed-metadata`) gets a rare-but-real case wrong — a small backward
/// reorder that happens to straddle the wrap boundary (e.g. previous raw `2`,
/// next raw `2^33 - 3`, a legitimate 5-tick backward step) is
/// indistinguishable, from an epoch-counter's point of view, from a huge
/// forward jump, and it reports the latter. Computing the delta first and
/// only then deciding whether it wrapped gets both directions right.
#[must_use]
pub fn unwrap_delta(prev_unwrapped: i128, prev_raw: u64, raw: u64) -> i128 {
    let mut delta = raw as i128 - prev_raw as i128;
    if delta > WRAP_33BIT_HALF as i128 {
        delta -= WRAP_33BIT as i128; // wrapped backward across 2^33
    } else if delta < -(WRAP_33BIT_HALF as i128) {
        delta += WRAP_33BIT as i128; // wrapped forward across 2^33
    }
    prev_unwrapped + delta
}

/// The modular forward distance from `from` to `to` on the 33-bit clock:
/// `(to - from) mod 2^33`, always in `[0, 2^33)`.
///
/// A distance greater than [`WRAP_33BIT_HALF`] means `to` is "behind" `from`
/// on the wrapped clock, not genuinely more than `2^32` ticks ahead — the
/// same wrap-vs-past ambiguity [`unwrap_delta`] resolves using history; this
/// function resolves it using only the half-range convention (no state),
/// which is enough when the caller already knows the two values are
/// supposed to be close in time.
#[must_use]
pub fn wrapping_forward_distance(from: u64, to: u64) -> u64 {
    to.wrapping_sub(from) % WRAP_33BIT
}

/// `(a + b) mod 2^33` — e.g. an SCTE-35 `pts_time` shifted by its
/// `pts_adjustment`: `add(WRAP_33BIT - 1, 1) == 0`, and inputs at or above
/// the modulus are reduced first, so `add(WRAP_33BIT + 3, 0) == 3`.
#[must_use]
pub fn add(a: u64, b: u64) -> u64 {
    let a = a % WRAP_33BIT;
    let b = b % WRAP_33BIT;
    // Each reduced operand is < 2^33, so the sum cannot overflow a u64.
    (a + b) % WRAP_33BIT
}

/// `(a + delta) mod 2^33` where `delta` may be negative; the result is always
/// in `[0, 2^33)` — e.g. stepping five ticks back past zero wraps forward:
/// `add_signed(5, -10) == WRAP_33BIT - 5`.
#[must_use]
pub fn add_signed(a: u64, delta: i64) -> u64 {
    // i128 intermediates: `delta` alone can be `i64::MIN`, and the sum must
    // not wrap before the Euclidean reduction.
    let sum = i128::from(a % WRAP_33BIT) + i128::from(delta);
    let reduced = sum.rem_euclid(i128::from(WRAP_33BIT));
    // `rem_euclid` by 2^33 is in `[0, 2^33)`, so the cast is lossless.
    reduced as u64
}

/// The shortest signed distance from `from` to `to` on the 2^33 circle, in
/// `(-2^32, 2^32]` — e.g. crossing the wrap forward reads as a small positive
/// step, `signed_distance(WRAP_33BIT - 10, 5) == 15`, while stepping back
/// across it reads negative, `signed_distance(5, WRAP_33BIT - 10) == -15`.
#[must_use]
pub fn signed_distance(from: u64, to: u64) -> i64 {
    let forward = wrapping_forward_distance(from % WRAP_33BIT, to % WRAP_33BIT);
    if forward <= WRAP_33BIT_HALF {
        forward as i64 // `forward <= 2^32`: lossless
    } else {
        let back = WRAP_33BIT - forward;
        // `back` is in `[1, 2^32)` here, so the negation cannot overflow.
        -(back as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwrap_delta_forward_wrap_advances_by_one_modulus() {
        // prev near the top of the range, next small: a legitimate forward
        // wrap of +8 ticks, not a ~2^33-tick backward jump.
        let prev_unwrapped = (WRAP_33BIT - 10) as i128;
        let got = unwrap_delta(prev_unwrapped, WRAP_33BIT - 10, 5);
        assert_eq!(got, prev_unwrapped + 15);
        assert_eq!(got, 5 + WRAP_33BIT as i128);
    }

    #[test]
    fn unwrap_delta_small_forward_step_is_identity_shift() {
        assert_eq!(unwrap_delta(1_000, 1_000, 2_000), 2_000);
    }

    #[test]
    fn unwrap_delta_small_backward_step_is_preserved_not_wrapped() {
        // Ordinary B-frame reordering: a small backward step within an
        // epoch must NOT be treated as a wrap.
        assert_eq!(unwrap_delta(2_000, 2_000, 1_995), 1_995);
    }

    /// MUTATION-PROOF: a reorder that straddles the origin (previous raw
    /// value small, next raw value near the top of the range, representing a
    /// genuine small *backward* step across 0) must unwrap to a small
    /// negative delta, not a huge forward jump. This is exactly the case a
    /// naive forward-only epoch counter (what `timed_metadata::Timeline`
    /// used before this module existed) gets wrong. Verified by temporarily
    /// deleting the `delta > WRAP_33BIT_HALF` branch below (so only forward
    /// wraps are corrected): this test then fails with `got = 2^33 - 3`
    /// instead of `-3`, confirming the branch is load-bearing. Restored.
    #[test]
    fn unwrap_delta_backward_reorder_across_origin_stays_small_and_negative() {
        let got = unwrap_delta(2, 2, WRAP_33BIT - 3);
        assert_eq!(got, -3);
    }

    #[test]
    fn wrapping_forward_distance_small_forward_is_small() {
        assert_eq!(wrapping_forward_distance(100, 105), 5);
    }

    #[test]
    fn wrapping_forward_distance_wraps_at_modulus() {
        assert_eq!(wrapping_forward_distance(WRAP_33BIT - 1, 0), 1);
    }

    #[test]
    fn wrapping_forward_distance_backward_step_is_large() {
        // A backward step of 5 reports as (modulus - 5): a huge forward
        // distance, which callers threshold against `WRAP_33BIT_HALF` to
        // classify as "actually behind", not "far ahead".
        let d = wrapping_forward_distance(105, 100);
        assert_eq!(d, WRAP_33BIT - 5);
        assert!(d > WRAP_33BIT_HALF);
    }

    #[test]
    fn add_wraps_at_modulus() {
        assert_eq!(add(WRAP_33BIT - 1, 1), 0);
        assert_eq!(add(WRAP_33BIT - 10, 20), 10);
    }

    #[test]
    fn add_reduces_inputs_at_or_above_the_modulus() {
        assert_eq!(add(WRAP_33BIT + 3, 0), 3);
    }

    #[test]
    fn add_signed_negative_delta_wraps_below_zero() {
        assert_eq!(add_signed(5, -10), WRAP_33BIT - 5);
    }

    #[test]
    fn add_signed_extreme_deltas_stay_in_range() {
        for delta in [i64::MIN, i64::MAX] {
            let got = add_signed(0, delta);
            assert!(
                got < WRAP_33BIT,
                "add_signed(0, {delta}) = {got} escaped [0, 2^33)"
            );
        }
    }

    #[test]
    fn signed_distance_matches_doc_examples() {
        assert_eq!(signed_distance(WRAP_33BIT - 10, 5), 15);
        assert_eq!(signed_distance(5, WRAP_33BIT - 10), -15);
    }

    #[test]
    fn signed_distance_half_range_boundary_is_positive() {
        // The range is (-2^32, 2^32]: exactly half the modulus reads as +2^32,
        // one tick past it flips to the negative shortest path.
        assert_eq!(signed_distance(0, WRAP_33BIT_HALF), WRAP_33BIT_HALF as i64);
        assert_eq!(
            signed_distance(0, WRAP_33BIT_HALF + 1),
            -((WRAP_33BIT_HALF - 1) as i64)
        );
    }

    #[test]
    fn signed_distance_reduces_inputs_at_or_above_the_modulus() {
        assert_eq!(signed_distance(WRAP_33BIT + 5, WRAP_33BIT + 8), 3);
    }
}
