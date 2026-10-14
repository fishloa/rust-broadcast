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
//! - [`pcr_forward_distance`] / [`pcr_signed_distance`] / [`pcr_from_parts`] /
//!   [`pcr_split`] — the same modular arithmetic for the 27 MHz PCR clock
//!   (`base * 300 + extension`, wrapping at `2^33 * 300`, which is *not* a power
//!   of two, so `wrapping_sub(..) % M` is wrong across a wrap).
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

// ── 27 MHz PCR clock (ISO/IEC 13818-1 §2.4.3.5) ─────────────────────────────

/// MPEG-2 Systems 90 kHz system-clock rate: the unit of PTS/DTS and of the
/// 33-bit PCR/ESCR/SCR *base* (ISO/IEC 13818-1 §2.4.3.5 / §2.4.3.7).
pub const PTS_HZ: u64 = 90_000;

/// The 27 MHz system-clock rate: the unit of a full `PCR` (`base * 300 +
/// extension`), ESCR and SCR value (ISO/IEC 13818-1 §2.4.2.1 / §2.4.3.5).
pub const PCR_HZ: u64 = 27_000_000;

/// `base * 300 + extension`: how many 27 MHz ticks one 90 kHz base tick spans
/// (`PCR_HZ / PTS_HZ`), and the modulus of the 9-bit `extension`.
pub const PCR_EXT_MODULUS: u64 = PCR_HZ / PTS_HZ;

/// The wrap period of a full PCR on the 27 MHz clock: `2^33 * 300` ticks
/// (~26.5 h). It is NOT a power of two, so a wrapped difference computed with
/// `u64::wrapping_sub(..) % PCR_WRAP_27MHZ` is wrong whenever the PCR wraps or
/// steps backward (2^64 is not a multiple of this modulus, so the u64 wrap
/// contributes a bogus `2^64 mod PCR_WRAP_27MHZ` offset): use
/// [`pcr_forward_distance`] / [`pcr_signed_distance`].
pub const PCR_WRAP_27MHZ: u64 = WRAP_33BIT * PCR_EXT_MODULUS;

/// A full 27 MHz clock value from its parts: `base * 300 + extension`
/// (ISO/IEC 13818-1 §2.4.3.5; the same arithmetic as ESCR §2.4.3.7 and the
/// program-stream SCR §2.5.3.4). Wrapping arithmetic: an out-of-range `base`
/// (wider than 33 bits) yields a meaningless value rather than a panic, like
/// every parser-fed helper in this crate.
#[must_use]
pub const fn pcr_from_parts(base: u64, extension: u16) -> u64 {
    base.wrapping_mul(PCR_EXT_MODULUS)
        .wrapping_add(extension as u64)
}

/// Split a 27 MHz clock value into `(base, extension)`: `base = ticks / 300`
/// truncated to its 33-bit wire width, `extension = ticks % 300` (always < 300,
/// so it fits the 9-bit field). Round-trips with [`pcr_from_parts`] for any
/// `ticks < PCR_WRAP_27MHZ`.
#[must_use]
pub const fn pcr_split(ticks: u64) -> (u64, u16) {
    (
        (ticks / PCR_EXT_MODULUS) & (WRAP_33BIT - 1),
        (ticks % PCR_EXT_MODULUS) as u16,
    )
}

/// The modular forward distance from `from` to `to` on the 27 MHz PCR clock:
/// `(to - from) mod 2^33*300`, always in `[0, PCR_WRAP_27MHZ)`. Inputs at or
/// above the modulus are reduced first. This is the PCR analogue of
/// [`wrapping_forward_distance`] and the only correct way to take a PCR delta
/// across a wrap (see [`PCR_WRAP_27MHZ`] for why `wrapping_sub` is not).
#[must_use]
pub fn pcr_forward_distance(from: u64, to: u64) -> u64 {
    // Both reduced operands are < M < 2^42, so `to + M - from` cannot overflow.
    (to % PCR_WRAP_27MHZ + PCR_WRAP_27MHZ - from % PCR_WRAP_27MHZ) % PCR_WRAP_27MHZ
}

/// The shortest signed distance from `from` to `to` on the PCR circle, in
/// `(-PCR_WRAP_27MHZ/2, PCR_WRAP_27MHZ/2]`: a forward wrap reads as a small
/// positive step, a small backward step (also across the wrap) as negative.
#[must_use]
pub fn pcr_signed_distance(from: u64, to: u64) -> i64 {
    let forward = pcr_forward_distance(from, to);
    if forward <= PCR_WRAP_27MHZ / 2 {
        forward as i64 // `forward <= M/2 < 2^41`: lossless
    } else {
        -((PCR_WRAP_27MHZ - forward) as i64) // in `[1, M/2)`: lossless
    }
}

// ── 5-byte marker-bit timestamp (PTS / DTS / DTS_next_AU) ───────────────────

/// All three `marker_bit`s of a 5-byte marked timestamp set to `1` (the value
/// every PES `PTS`/`DTS` must carry, ISO/IEC 13818-1 §2.4.3.7).
pub const MARKERS_ALL_SET: u8 = 0b111;

/// A decoded 5-byte marker-bit timestamp field.
///
/// Wire layout (ISO/IEC 13818-1 §2.4.3.7 Table 2-21 `PTS`/`DTS`; §2.4.3.5
/// `DTS_next_AU` uses the same shape): `prefix(4) ts[32:30](3) marker(1)
/// ts[29:15](15) marker(1) ts[14:0](15) marker(1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marked {
    /// The leading 4 bits (`0b0010` PTS-only, `0b0011` PTS-with-DTS, `0b0001`
    /// DTS, or a seamless-splice `splice_type`).
    pub prefix: u8,
    /// The 33-bit timestamp.
    pub timestamp: u64,
    /// The three `marker_bit`s: bit 2 = byte 0 bit 0, bit 1 = byte 2 bit 0,
    /// bit 0 = byte 4 bit 0. A conformant field is [`MARKERS_ALL_SET`].
    pub markers: u8,
}

/// Decode a 5-byte marker-bit timestamp field. Never fails: the caller
/// validates `prefix` and `markers` against its own spec clause and maps a
/// mismatch into its own error type.
#[must_use]
pub fn unpack_marked(b: &[u8; 5]) -> Marked {
    let hi = u64::from((b[0] >> 1) & 0x07); // [32:30]
    let mid = (u64::from(b[1]) << 7) | u64::from(b[2] >> 1); // [29:15]
    let lo = (u64::from(b[3]) << 7) | u64::from(b[4] >> 1); // [14:0]
    Marked {
        prefix: b[0] >> 4,
        timestamp: (hi << 30) | (mid << 15) | lo,
        markers: ((b[0] & 1) << 2) | ((b[2] & 1) << 1) | (b[4] & 1),
    }
}

/// Encode a 5-byte marker-bit timestamp field with explicit marker bits (see
/// [`Marked::markers`]); `timestamp` is reduced modulo 2^33 and only the low
/// nibble of `prefix` / low 3 bits of `markers` are used (the wire widths).
#[must_use]
pub fn pack_marked_with(timestamp: u64, prefix: u8, markers: u8) -> [u8; 5] {
    let ts = timestamp & (WRAP_33BIT - 1);
    [
        ((prefix & 0x0F) << 4) | ((((ts >> 30) & 0x07) as u8) << 1) | ((markers >> 2) & 1),
        ((ts >> 22) & 0xFF) as u8,
        ((((ts >> 15) & 0x7F) as u8) << 1) | ((markers >> 1) & 1),
        ((ts >> 7) & 0xFF) as u8,
        (((ts & 0x7F) as u8) << 1) | (markers & 1),
    ]
}

/// Encode a conformant PTS/DTS field: every `marker_bit` set to `1`.
#[must_use]
pub fn pack_marked(timestamp: u64, prefix: u8) -> [u8; 5] {
    pack_marked_with(timestamp, prefix, MARKERS_ALL_SET)
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
    #[test]
    fn pcr_constants_are_the_spec_values() {
        assert_eq!(PCR_EXT_MODULUS, 300);
        assert_eq!(PCR_WRAP_27MHZ, (1u64 << 33) * 300);
        assert_eq!(PCR_WRAP_27MHZ, 2_576_980_377_600);
        assert_eq!(PCR_HZ, PTS_HZ * PCR_EXT_MODULUS);
    }

    #[test]
    fn pcr_parts_round_trip() {
        for ticks in [0, 1, 299, 300, 301, 27_000_000, PCR_WRAP_27MHZ - 1] {
            let (base, ext) = pcr_split(ticks);
            assert!(ext < 300 && base < WRAP_33BIT);
            assert_eq!(pcr_from_parts(base, ext), ticks);
        }
        // An over-wide base wraps instead of panicking.
        let _ = pcr_from_parts(u64::MAX, 299);
    }

    #[test]
    fn pcr_forward_distance_across_the_wrap() {
        let m = PCR_WRAP_27MHZ;
        assert_eq!(pcr_forward_distance(m - 5, 10), 15);
        assert_eq!(pcr_forward_distance(10, m - 5), m - 15);
        assert_eq!(pcr_forward_distance(7, 7), 0);
        assert_eq!(pcr_forward_distance(0, m - 1), m - 1);
        // Inputs at or above the modulus are reduced first.
        assert_eq!(pcr_forward_distance(m + 3, 5), 2);
    }

    /// The bug this exists to prevent: `to.wrapping_sub(from) % M` is wrong
    /// whenever `to < from`, because 2^64 mod M != 0.
    #[test]
    fn pcr_forward_distance_is_not_wrapping_sub_mod_m() {
        let m = PCR_WRAP_27MHZ;
        let (from, to) = (m - 5, 10u64);
        let naive = to.wrapping_sub(from) % m;
        assert_ne!(naive, 15, "the naive form is wrong across a wrap");
        assert_eq!(pcr_forward_distance(from, to), 15);
    }

    #[test]
    fn pcr_signed_distance_is_the_shortest_path() {
        let m = PCR_WRAP_27MHZ;
        assert_eq!(pcr_signed_distance(m - 10, 5), 15);
        assert_eq!(pcr_signed_distance(5, m - 10), -15);
        assert_eq!(pcr_signed_distance(0, m / 2), (m / 2) as i64);
        assert_eq!(pcr_signed_distance(0, m / 2 + 1), -((m / 2 - 1) as i64));
        assert_eq!(pcr_signed_distance(100, 100), 0);
    }
    /// A REAL 5-byte PTS field: the first PES of `fixtures/ts/m6-single.ts`
    /// (private_stream_1, `PTS_DTS_flags = 10`). The expected value was decoded
    /// independently by slicing the 40-bit string as
    /// `prefix(4) ts[32:30](3) '1' ts[29:15](15) '1' ts[14:0](15) '1'`.
    #[test]
    fn marked_decodes_and_re_encodes_a_real_capture_pts_field() {
        let real = [0x29, 0xFD, 0x91, 0xD2, 0x29];
        let m = unpack_marked(&real);
        assert_eq!(
            m,
            Marked {
                prefix: 0b0010,
                timestamp: 5_358_512_404,
                markers: MARKERS_ALL_SET
            }
        );
        assert_eq!(pack_marked(m.timestamp, m.prefix), real);
    }

    #[test]
    fn marked_round_trips_every_boundary_and_marker_combination() {
        for ts in [0, 1, 90_000, 0x1_2345_6789, WRAP_33BIT - 1] {
            for prefix in [0b0001, 0b0010, 0b0011, 0x0F, 0] {
                for markers in 0..8u8 {
                    let b = pack_marked_with(ts, prefix, markers);
                    let m = unpack_marked(&b);
                    assert_eq!(
                        (m.prefix, m.timestamp, m.markers),
                        (prefix, ts, markers),
                        "ts={ts:#x} prefix={prefix:#x} markers={markers:#05b}"
                    );
                }
            }
        }
    }

    #[test]
    fn marked_pack_reduces_modulo_2_pow_33_and_masks_the_prefix() {
        assert_eq!(pack_marked(WRAP_33BIT + 5, 0b0010), pack_marked(5, 0b0010));
        // Only the low nibble of the prefix is on the wire.
        assert_eq!(pack_marked(7, 0xF2), pack_marked(7, 0x02));
    }

    #[test]
    fn marked_a_cleared_marker_is_visible_to_the_caller() {
        let mut b = pack_marked(0x1234, 0b0010);
        b[2] &= !1; // clear the middle marker bit
        assert_eq!(unpack_marked(&b).markers, 0b101);
    }
}
