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
}
