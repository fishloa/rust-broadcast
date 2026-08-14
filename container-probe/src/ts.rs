//! MPEG-2 Transport Stream prober — ISO/IEC 13818-1 §2.4.
//!
//! A **lattice** search rather than fixed-offset checks. For each candidate
//! stride (188, 192, 204, 208) and each phase offset `0..stride` (bounded by
//! the stride itself — one phase per byte position within a packet), we walk
//! `phase + n*stride` and count the longest run of consecutive `TS_SYNC_BYTE`s.
//! The `(stride, phase)` pair with the longest run wins.
//!
//! - `>= 8` confirmations -> [`crate::Confidence::LATTICE_STRONG`].
//! - `3..=7` -> [`crate::Confidence::LATTICE_WEAK`].
//! - `1..=2` on a consistent lattice too short to reach 8 -> `Insufficient`.
//! - no sync at all -> no match, but only `Unknown` once the region is a full
//!   packet or more: a region shorter than one packet has not examined a full
//!   packet yet, so it answers `Insufficient` ("read more") instead.
//!
//! The phase scan is what handles a capture beginning mid-packet — the case
//! today's fixed-offset implementation fails. A cheap precondition (no
//! `TS_SYNC_BYTE` anywhere in the first stride window) skips the phase loop
//! entirely for obviously non-TS input.
//!
//! Strides: **188** (ISO/IEC 13818-1), **192** (M2TS/BDAV — a 4-byte
//! `TP_extra_header` timestamp prefix per 188-byte packet, Blu-ray Disc
//! Association), **204** (DVB with Reed-Solomon parity, ETSI EN 300 421/300
//! 744), **208** (a `TP_extra_header`-prefixed 204 variant seen from some
//! TS-over-IP recorders).

use crate::{Confidence, Detail::Ts, Evidence, Outcome};

/// The sync byte that begins every TS packet (ISO/IEC 13818-1 §2.4.3.3).
const TS_SYNC_BYTE: u8 = 0x47;

/// The nominal MPEG-2 TS packet length in bytes (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_SIZE: usize = 188;

/// M2TS/BDAV packet length: a `188`-byte TS packet preceded by a 4-byte
/// `TP_extra_header` (copy-permission + arrival-time-stamp); an M2TS file is
/// exactly these 192-byte records back to back.
const TS_PACKET_SIZE_192: usize = 192;

/// A DVB TS packet with a trailing 16-byte Reed-Solomon parity block (ETSI
/// EN 300 421 §5.2.2 — `188` payload + `16` parity).
const TS_PACKET_SIZE_204: usize = 204;

/// A `204`-byte DVB packet with a `TP_extra_header` prefix, 208 bytes (an
/// M2TS-style wrapper on the RS-protected packet, from some recorders).
const TS_PACKET_SIZE_208: usize = 208;

/// Consecutive sync confirmations that lift a TS lattice to
/// [`crate::Confidence::LATTICE_STRONG`] (design §"MPEG-2 TS").
const TS_CONFIRM_FOR_STRONG: usize = 8;
/// Minimum consecutive sync confirmations for a positive match; below this the
/// lattice is either noise or too short to prove.
const TS_CONFIRM_FOR_WEAK: usize = 3;
/// Minimum sync coverage on a candidate lane, as a percentage of that lane's
/// available positions, for the lane to be accepted.
///
/// Run length alone (`longest >= TS_CONFIRM_FOR_WEAK`) is not evidence: across
/// a high-entropy buffer (e.g. a CENC-encrypted MP4 such as
/// `fixtures/mp4/cenc.mp4`) three consecutive `0x47` bytes can align on one of
/// the 792 lanes purely by chance, producing a confident false `MpegTs`. The
/// real discriminator is **coverage** — what fraction of a lane's positions
/// actually hold a sync byte: a genuine TS stream syncs at essentially every
/// position (~100%), while random noise hits ~3 in ~117 (~2.5%). Requiring
/// coverage `>= 50%` cleanly separates the two while tolerating mid-file
/// discontinuities or corruption in a real capture (most positions remain
/// syncs). `cenc.mp4` was the real file that forced this rule.
const TS_MIN_COVERAGE_PCT: u64 = 50;
/// The four candidate packet strides — the legal MPEG-2 TS/DVB/M2TS lengths.
const TS_STRIDES: [usize; 4] = [
    TS_PACKET_SIZE,
    TS_PACKET_SIZE_192,
    TS_PACKET_SIZE_204,
    TS_PACKET_SIZE_208,
];

/// The registered TS prober: lattice search over `limit` bytes.
///
/// Returns `Outcome::Match` with `Detail::Ts { stride, phase }` and a tiered
/// confidence, `Outcome::Insufficient` when the buffer was too short to reach
/// [`TS_CONFIRM_FOR_STRONG`] at a lattice that is consistent so far, or
/// `Outcome::None`.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    // A buffer that is uniformly the sync byte carries no structural evidence:
    // `0x47` is "G", and a continuous run with no packet content to fill the
    // lanes is the pathological TS-shaped case, not a real stream. Real files
    // always contain non-sync payload bytes.
    //
    // This rejection only fires once the region is a full packet or more: a
    // sub-packet all-`0x47` region is a *truncated* read — it cannot yet have
    // shown the payload bytes a real packet holds, and it is at least as
    // plausible a TS prefix as any other byte pattern. Routing it through the
    // lattice (as every other short region does) yields `Insufficient`, not
    // `Unknown`, so a streaming caller is told "read more" rather than "stop".
    if region.len() >= TS_PACKET_SIZE && region.iter().all(|&b| b == TS_SYNC_BYTE) {
        return Outcome::None;
    }

    // Best (stride, phase): the longest consecutive sync run seen over all
    // lanes. Track it here so we can report its tier and detail.
    let mut best_stride: Option<usize> = None;
    let mut best_phase = 0usize;
    let mut best_run = 0usize;
    // The single longest run seen across *all* lanes, regardless of whether it
    // reached the weak threshold. Used to distinguish `Insufficient` (a
    // coherent partial lattice a longer buffer could confirm) from `None`.
    let mut best_observed_run = 0usize;
    let mut best_observed_stride = 0usize;
    let mut best_observed_phase = 0usize;

    for &stride in &TS_STRIDES {
        let win = core::cmp::min(stride, region.len());
        // Cheap precondition: if no sync byte appears anywhere in the first
        // stride window, skip this stride's phase loop entirely.
        if !region[..win].contains(&TS_SYNC_BYTE) {
            continue;
        }

        for phase in 0..stride {
            let mut run = 0usize;
            let mut longest = 0usize;
            let mut total_syncs = 0usize;
            let mut pos = phase;
            while pos < region.len() {
                if region[pos] == TS_SYNC_BYTE {
                    run += 1;
                    total_syncs += 1;
                    if run > longest {
                        longest = run;
                    }
                } else {
                    run = 0;
                }
                pos += stride;
            }
            // Track the best partial run of any length for the
            // `Insufficient` / `None` decision.
            if longest > best_observed_run {
                best_observed_run = longest;
                best_observed_stride = stride;
                best_observed_phase = phase;
            }
            // A candidate lane must meet BOTH the run-length test AND cover half
            // its positions with sync bytes (see `TS_MIN_COVERAGE_PCT`).
            let possible = region.len().saturating_sub(phase).div_ceil(stride);
            let coverage = if possible == 0 {
                0
            } else {
                (total_syncs as u64 * 100) / possible as u64
            };
            if longest >= TS_CONFIRM_FOR_WEAK && coverage >= TS_MIN_COVERAGE_PCT {
                let better = match best_stride {
                    None => true,
                    Some(s) => longest > best_run || (longest == best_run && stride < s),
                };
                if better {
                    best_stride = Some(stride);
                    best_phase = phase;
                    best_run = longest;
                }
            }
        }
    }

    let stride = match best_stride {
        Some(s) => s,
        None => {
            // No lane reached the weak threshold.
            if best_observed_run == 0 {
                // No sync byte anywhere in the region. That only proves "not
                // TS" if the region was long enough that a real TS *must* have
                // shown a sync: a TS stream syncs at least once every smallest
                // stride (188 bytes), so a region of `TS_PACKET_SIZE` bytes or
                // more with no sync byte at all can never be TS. A region
                // shorter than one packet has simply not examined a full packet
                // yet — e.g. `ts_midpacket_phase.ts` holds its first `0x47` at
                // offset 111, so every prefix `16..=111` contains no sync but is
                // still a prefix of a genuine TS. That is `Insufficient`, not
                // `None`: the shared `Insufficient` vs `Unknown` decision.
                return crate::ran_out_or_ruled_out(region.len() < TS_PACKET_SIZE, need_at_least());
            }
            // A coherent partial lattice exists (>= 1 sync). It is
            // `Insufficient` only if the region is too short to have proven
            // `TS_CONFIRM_FOR_STRONG` at the best partial lane — i.e. a longer
            // buffer could still confirm it. Once the region is long enough to
            // prove that lane outright yet nothing confirmed, the answer is
            // definitively `None`: `0x47` is "G", a stray byte, and will not
            // become a transport stream by reading further.
            let could_prove =
                best_observed_phase + TS_CONFIRM_FOR_STRONG * best_observed_stride <= region.len();
            if could_prove {
                return Outcome::None;
            }
            return Outcome::Insufficient(need_at_least());
        }
    };

    // A lane cleared BOTH the run-length and coverage tests, so it is a match.
    // The run length picks the tier and nothing downgrades it to
    // `Insufficient`: a short buffer whose every lattice position is a sync
    // byte has *matched* — weakly, which is exactly what `LATTICE_WEAK` is for
    // (design §"Confidence model": 3-7 confirmations IS a match).
    //
    // An earlier revision additionally required the region to be long enough to
    // reach `TS_CONFIRM_FOR_STRONG` at this lane, and reported `Insufficient`
    // otherwise. That was wrong, and a corpus sweep caught it: eleven real,
    // *complete* TS files of 188 B - 1.1 KB (`fixtures/ts/scte35-*.ts`,
    // `fixtures/ts/pts-*.ts`, `fixtures/mpeg-ts/af-*.ts`) each answered
    // "supply more bytes" for a file the caller had already read to the end —
    // the same false-`Insufficient` class of bug as the sync-byte-presence
    // check this module used to make. `Insufficient` belongs to the
    // no-qualifying-lane path above, never here.
    let detail = Ts {
        stride: stride as u16,
        phase: best_phase as u16,
    };
    let confidence = if best_run >= TS_CONFIRM_FOR_STRONG {
        Confidence::LATTICE_STRONG
    } else {
        Confidence::LATTICE_WEAK
    };
    Outcome::Match(Evidence { confidence, detail })
}

/// A lower bound on bytes that could resolve the verdict: strictly more than
/// the bytes actually examined (`have` = the budget-capped region length), and
/// at least enough for `TS_CONFIRM_FOR_STRONG` whole packets at the smallest
/// (188-byte) stride.
///
/// Base this on the *examined* region length, not the caller's full buffer:
/// `probe_with_budget(&ten_mb_ts, 64)` examined only 64 bytes, so its
/// `need_at_least` must answer "supply more than 64", never "supply more than
/// 10 000 000" — the budget, not the buffer, was the constraint.
fn need_at_least() -> usize {
    // Structural only. The `max(..., have + TS_PACKET_SIZE)` this replaces made
    // the answer track the buffer: once `have` overtook the floor it grew 188
    // bytes a turn, so a caller crossed a 256 KiB file in 1394 reads. Strict
    // progress and geometric convergence are guaranteed centrally by
    // `crate::normalise_need`, so bounding it here added nothing and destroyed
    // the only useful part of the answer.
    TS_PACKET_SIZE * TS_CONFIRM_FOR_STRONG
}

#[cfg(test)]
mod all_sync {
    //! The uniform-`0x47` rejection guard: a short all-sync region is a
    //! *truncated* read and must answer `Insufficient`, exactly as a short
    //! region of any other single byte does — a sub-packet all-`0x47` buffer is
    //! at least as plausible a TS prefix as a sub-packet all-`'A'` buffer.

    use super::*;

    /// A prober's `need_at_least` must be **structural** -- a property of what
    /// it is looking for, not of how many bytes it happened to be given.
    ///
    /// Asserted at the prober, because `crate::normalise_need`'s floor masks it
    /// end-to-end: an audit reverted the equivalent fix in another prober and
    /// the whole-crate suite stayed green. A guard has to sit where the value
    /// is produced, or the layer above hides the defect.
    ///
    /// MUTATION VERIFIED: restoring `max(FLOOR, have + UNIT)` makes the need
    /// grow with the buffer and fails this with a non-constant list.
    #[test]
    fn the_ran_out_need_does_not_track_the_buffer_length() {
        let mut seen: std::vec::Vec<usize> = std::vec::Vec::new();
        for len in [4usize, 64, 200, 700, 1400] {
            let buf = {
                let mut b = std::vec![0x47u8];
                b.resize(len, 0x00);
                b
            };
            if let Outcome::Insufficient(need) = probe(&buf, buf.len()) {
                seen.push(need);
            }
        }
        assert!(
            seen.len() >= 2,
            "the seed must reach Insufficient at two or more lengths, else this \
             guard proves nothing (got {seen:?})"
        );
        assert!(
            seen.windows(2).all(|w| w[0] == w[1]),
            "need_at_least must be identical at every length short of the \
             structure it names, got {seen:?} -- a value that grows with the \
             buffer makes the caller crawl"
        );
    }

    /// A 187-byte all-`0x47` region (one byte short of a full packet) must be
    /// `Insufficient`, not `Unknown`: it has not examined a full packet yet, and
    /// `0x47` is a *more* plausible TS prefix than `'A'`, so "stop" would be
    /// backwards. Before the guard was length-capped, `probe(&[0x47; 187])`
    /// returned `Unknown` while `probe(&[b'A'; 187])` returned `Insufficient`.
    ///
    /// Observed under the mutation (the `region.len() >= TS_PACKET_SIZE`
    /// condition removed, so the guard fires on any length):
    /// ```
    /// all-0x47 sub-packet region must be Insufficient, got None
    /// ```
    #[test]
    fn all_sync_sub_packet_region_is_insufficient() {
        match probe(&[TS_SYNC_BYTE; TS_PACKET_SIZE - 1], TS_PACKET_SIZE - 1) {
            Outcome::Insufficient(_) => {}
            other => panic!("all-0x47 sub-packet region must be Insufficient, got {other:?}"),
        }
    }

    /// The same length of any other byte is `Insufficient` too — the two cases
    /// are now consistent.
    #[test]
    fn non_sync_sub_packet_region_is_insufficient() {
        match probe(&[b'A'; TS_PACKET_SIZE - 1], TS_PACKET_SIZE - 1) {
            Outcome::Insufficient(_) => {}
            other => panic!("all-'A' sub-packet region must be Insufficient, got {other:?}"),
        }
    }

    /// A large all-`0x47` buffer (a full packet or more) is still rejected:
    /// every lane would score a false lattice, and no real stream is a uniform
    /// run of one byte.
    #[test]
    fn all_sync_full_region_is_still_rejected() {
        match probe(&[TS_SYNC_BYTE; 4096], 4096) {
            Outcome::None => {}
            other => panic!("4096-byte all-0x47 region must be None, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod drift {
    //! Pins this module's re-declared constants to the upstream crate that
    //! already validates them against real fixtures.
    //!
    //! These live here, not in `tests/drift_guard.rs`, because the constants
    //! are private to this module. An integration test cannot see them, so it
    //! can only compare upstream against a *literal* — which catches upstream
    //! changing but NOT this crate's copy drifting, the direction that actually
    //! matters. Verified: with the guard in `tests/`, editing `TS_SYNC_BYTE` to
    //! `0x48` left it green. A unit test sees the real constant.

    /// ISO/IEC 13818-1 §2.4.3.3 sync byte, pinned to `mpeg-ts`.
    #[test]
    fn sync_byte_matches_mpeg_ts() {
        assert_eq!(
            super::TS_SYNC_BYTE,
            mpeg_ts::ts::TS_SYNC_BYTE,
            "container-probe's TS_SYNC_BYTE has drifted from mpeg-ts's"
        );
    }

    /// ISO/IEC 13818-1 §2.4.3.2 packet length, pinned to `mpeg-ts`.
    #[test]
    fn packet_size_matches_mpeg_ts() {
        assert_eq!(
            super::TS_PACKET_SIZE,
            mpeg_ts::ts::TS_PACKET_SIZE,
            "container-probe's TS_PACKET_SIZE has drifted from mpeg-ts's"
        );
    }

    /// The M2TS/BDAV and DVB strides are the 188-byte packet plus their
    /// respective wrappers, so they must stay derived from it rather than
    /// becoming independent literals.
    #[test]
    fn wrapped_strides_stay_derived_from_the_packet_size() {
        const TP_EXTRA_HEADER: usize = 4;
        const RS_PARITY: usize = 16;
        assert_eq!(
            super::TS_PACKET_SIZE_192,
            super::TS_PACKET_SIZE + TP_EXTRA_HEADER
        );
        assert_eq!(super::TS_PACKET_SIZE_204, super::TS_PACKET_SIZE + RS_PARITY);
        assert_eq!(
            super::TS_PACKET_SIZE_208,
            super::TS_PACKET_SIZE + RS_PARITY + TP_EXTRA_HEADER
        );
    }
}
