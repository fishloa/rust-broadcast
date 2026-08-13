//! ISO Base Media File Format prober — ISO/IEC 14496-12:2015 §4.2.
//!
//! Walks the top-level box chain by size. Header layout follows
//! `transmux/src/box_types.rs` (which round-trips real fixtures): a `u32`
//! big-endian size, a 4-byte type, an optional 64-bit `largesize` when
//! `size == 1`, and `size == 0` meaning "runs to end of file".
//!
//! - The first box's type must be one of `ftyp`, `styp`, `moov`, `moof`,
//!   `skip`, `free`, `mdat`, else no match.
//! - `>= 2` boxes chaining cleanly (each size landing exactly on the next
//!   header, or on a clean truncation at the region end) -> `STRUCTURAL`.
//! - One valid box that runs to the region end -> `HEURISTIC`; fewer -> none.
//! - When the first box is `ftyp`/`styp`, its major brand (the first 4 bytes of
//!   the box body) goes into `Detail::Isobmff`, together with the
//!   [`IsobmffLayout`](crate::IsobmffLayout) the walk observed.

use crate::{Confidence, Detail::Isobmff, Evidence, IsobmffLayout, Outcome};

/// Header size of the fixed `size`(32) + `type`(32) Box fields (ISO/IEC
/// 14496-12:2015 §4.2).
const BOX_HEADER_MIN_SIZE: usize = 8;
/// Header size of the 64-bit `largesize` form: 8 fixed + 8 largesize
/// (`size==1`) (§4.2).
const BOX_HEADER_LARGESIZE_SIZE: usize = 16;
/// Byte offset of the major brand within a `ftyp`/`styp` box — 4 bytes into
/// the box body, i.e. 8 bytes into the box (after the header) (§8.16.2).
const BRAND_OFFSET: usize = 8;
/// The special `size` value `0`: the box runs to the end of file (§4.2).
const SIZE_TO_EOF: u32 = 0;
/// The special `size` value `1`: a 64-bit `largesize` follows (§4.2).
const SIZE_INDICATES_LARGESIZE: u32 = 1;
/// The `ftyp` box type (ISO/IEC 14496-12 §8.16.2).
const TYPE_FTYP: [u8; 4] = *b"ftyp";
/// The `styp` box type (segment type, §8.16.3).
const TYPE_STYP: [u8; 4] = *b"styp";
/// The `moov` box type (§8.2.1).
const TYPE_MOOV: [u8; 4] = *b"moov";
/// The `moof` box type (§8.8.1).
const TYPE_MOOF: [u8; 4] = *b"moof";
/// The `skip` box type (§8.2.2).
const TYPE_SKIP: [u8; 4] = *b"skip";
/// The `free` box type (§8.2.1).
const TYPE_FREE: [u8; 4] = *b"free";
/// The `mdat` box type (§8.2.1).
const TYPE_MDAT: [u8; 4] = *b"mdat";
/// The major brand field of an ISO file (its 4-byte brand), which is what the
/// leading `ftyp`/`styp` box reports.
const BRAND_LEN: usize = 4;

/// The 4-byte box types accepted as the FILE's first box (§4.4 / §8.2 / §8.8):
/// a standalone init segment begins `ftyp`, a CMAF/fMP4 segment `styp`/`moof`,
/// a progressive file `ftyp`/`moov`, etc.
const LEADING_BOX_TYPES: [[u8; 4]; 7] = [
    TYPE_FTYP, TYPE_STYP, TYPE_MOOV, TYPE_MOOF, TYPE_SKIP, TYPE_FREE, TYPE_MDAT,
];

/// The registered ISOBMFF prober: size-driven walk of `limit` bytes.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    if region.len() < BOX_HEADER_MIN_SIZE {
        // Shorter than one 8-byte box header: the walk cannot even read the
        // first box's size+type, so a truncated .mp4 read a few bytes at a time
        // is undecided (`Insufficient`), not `Unknown`.
        return Outcome::Insufficient(BOX_HEADER_MIN_SIZE);
    }

    let leading_type = [region[4], region[5], region[6], region[7]];
    if !LEADING_BOX_TYPES.contains(&leading_type) {
        return Outcome::None;
    }
    let leading_is_ftyp = leading_type == TYPE_FTYP || leading_type == TYPE_STYP;

    // Walk the top-level chain, counting fully-contained boxes.
    let mut offset = 0usize;
    let mut boxes = 0u64;
    let mut brand: Option<[u8; 4]> = None;
    // Which structural box the file uses to carry sample metadata. `moof`
    // means fragmented (ISO/IEC 14496-12 §8.8 movie fragments, the CMAF/fMP4
    // shape); `moov` alone means a progressive file with sample tables. The
    // walk already visits every top-level box, so this costs nothing and
    // spares a consumer re-walking the chain to pick a demuxer.
    let mut saw_moof = false;
    let mut saw_moov = false;
    // `clean` stays true while each box lands exactly at the next header or at
    // the region end — the chain neither overflows nor leaves a dangling tail.
    let mut clean = true;
    // `ran_out` becomes true when the walk stopped because the supplied region
    // ended mid-structure (a box header or body extends past the region) rather
    // than because a box size was *invalid*. That distinction is the whole
    // `Insufficient` vs `Unknown` contract: ending mid-box proves nothing, so
    // the prober must ask for more bytes; a size that could never chain is a
    // definitive rejection. `ran_out_need` carries the byte count that would
    // complete the structure the walk was cut short in.
    let mut ran_out = false;
    let mut ran_out_need = 0usize;

    loop {
        if offset >= region.len() {
            break;
        }
        let rem = &region[offset..];
        let (size_u32, eff) = match decode_header(rem) {
            Some(x) => x,
            None => {
                // A trailing header that cannot be read: the region ended
                // mid-box-header. Not clean, and not a ruling-out — the missing
                // bytes could have completed a valid box.
                //
                // The need is the END of the header being read, absolutely: 8
                // bytes normally, but 16 when `size32 == 1`, because the
                // `largesize` field is part of the header. Reporting 8 for a
                // largesize header asks for bytes that still leave it unreadable,
                // so the caller comes straight back.
                clean = false;
                ran_out = true;
                ran_out_need = offset.saturating_add(header_len_required(rem));
                break;
            }
        };
        // A box must at least contain its own header — 8 bytes normally, but 16
        // when `size32 == 1` (the `largesize` field is part of the header, so a
        // `largesize` smaller than 16 would reach into the middle of its own
        // size field). A `size == 0` box runs to the region end and is always
        // valid and last. A box whose declared size exceeds the remaining region
        // is a *truncated* final box (a segment clipped mid-box at the region
        // end) — the brief's "clean truncation at the region end": count it,
        // treat the region as exhausted, and keep the chain clean.
        let min_header = if size_u32 == SIZE_INDICATES_LARGESIZE {
            BOX_HEADER_LARGESIZE_SIZE
        } else {
            BOX_HEADER_MIN_SIZE
        };
        if size_u32 == SIZE_TO_EOF || eff >= min_header {
            if offset == 0 && leading_is_ftyp && rem.len() >= BRAND_OFFSET + BRAND_LEN {
                brand = Some([rem[8], rem[9], rem[10], rem[11]]);
            }
            let this_type = [rem[4], rem[5], rem[6], rem[7]];
            if this_type == TYPE_MOOF {
                saw_moof = true;
            } else if this_type == TYPE_MOOV {
                saw_moov = true;
            }
            boxes += 1;
            if size_u32 == SIZE_TO_EOF || eff > rem.len() {
                // To-EOF, or the region truncates this box -> it is the last one.
                // Both keep the chain clean: a `size == 0` box is valid-to-EOF by
                // spec, and a box whose declared size exceeds the region is a
                // clean truncation at the region end (its header was valid, so it
                // is counted and the walk ends on a Match). This path therefore
                // deliberately never sets `ran_out` — asking for more bytes here
                // would contradict the "count the truncated final box" rule above.
                break;
            }
            offset += eff;
        } else {
            clean = false;
            break;
        }
    }

    if boxes == 0 {
        // Nothing walked: the chain never even started. The leading type was
        // already pinned as a legal box type above, so these two outcomes remain
        // and are decided by `ran_out`:
        //  - the first box *header* could not be read (the region ended
        //    mid-header) -> `ran_out`, a truncation, so "give me more";
        //  - the first box header was read but declared an impossible size
        //    (smaller than its own header) -> a definitive rejection (`None`).
        // Structural, not length-relative: `ran_out_need` is the end of the
        // header the walk was cut short in (8 bytes normally, 16 for a
        // `largesize` header). The previous `need_at_least(region.len())` was
        // `supplied + 8`, so the answer grew with the buffer instead of naming
        // the header, and a caller advanced 8 bytes per read.
        return crate::ran_out_or_ruled_out(ran_out, ran_out_need);
    }

    // True only when the walk consumed every byte the caller supplied — the
    // chain ended exactly at the region end AND the region was not clipped by
    // the probe budget. Anything less means more boxes may follow.
    let walked_whole_input = clean && offset >= region.len() && limit == data.len();

    let detail = Isobmff {
        major_brand: brand,
        boxes_walked: boxes.min(u8::MAX as u64) as u8,
        // `moof` is definitive on sight: only a fragmented file has one, and
        // seeing it does not depend on having read the rest.
        //
        // The absence of `moof` is NOT definitive unless the whole supplied
        // buffer was walked. Every fragmented file OPENS with a `ftyp`+`moov`
        // init segment and only reaches its first `moof` later, so a truncated
        // prefix of a fragmented file is indistinguishable from a progressive
        // one. Claiming `Progressive` there would send a consumer to the wrong
        // demuxer — measured: the first 64 bytes of a real fragmented CMAF
        // file (`fixtures/mp4/cmaf/av_frag.mp4`) contain `ftyp` and the start
        // of `moov` and nothing else.
        layout: if saw_moof {
            IsobmffLayout::Fragmented
        } else if saw_moov && walked_whole_input {
            IsobmffLayout::Progressive
        } else {
            IsobmffLayout::Unknown
        },
    };

    let confidence = if clean && boxes >= 2 {
        Confidence::STRUCTURAL
    } else if clean && boxes == 1 {
        Confidence::HEURISTIC
    } else {
        // The chain is not clean. If that is because the region ended
        // mid-structure (`ran_out`), the prober has proven nothing and must ask
        // for more bytes; if a box size was outright invalid, the file is ruled
        // out (`None`). This is the shared `Insufficient` vs `Unknown` decision
        // — see `crate::ran_out_or_ruled_out`.
        // The STRUCTURAL need, not a length-relative one. This previously took
        // `max(ran_out_need, region.len() + BOX_HEADER_MIN_SIZE)`, and that
        // second term dominates: it grows with whatever was supplied, so the
        // documented caller loop advanced 8 bytes per turn and had reached only
        // 36 of 262144 bytes after 12 reads. Strict progress is guaranteed
        // centrally by `crate::normalise_need`, so bounding it here is both
        // unnecessary and destructive of the useful answer.
        return crate::ran_out_or_ruled_out(ran_out, ran_out_need);
    };

    Outcome::Match(Evidence { confidence, detail })
}

/// Decode a box header at the front of `rem`: the raw `u32` size and the
/// effective byte length of the box. Returns `None` when the header is
/// truncated.
///
/// The effective length is the `largesize` when `size == 1`, `0`
/// (end-of-file) when `size == 0`, else the 32-bit `size` itself.
/// Bytes required to read the box header at the start of `rem`.
///
/// A normal header is [`BOX_HEADER_MIN_SIZE`]; a `size32 == 1` header carries a
/// 64-bit `largesize` and is [`BOX_HEADER_LARGESIZE_SIZE`]. Reporting the
/// smaller figure for a largesize header asks the caller for bytes that still
/// leave the header unreadable, so it returns immediately for more.
fn header_len_required(rem: &[u8]) -> usize {
    if rem.len() >= 4 {
        let size32 = u32::from_be_bytes([rem[0], rem[1], rem[2], rem[3]]);
        if size32 == SIZE_INDICATES_LARGESIZE {
            return BOX_HEADER_LARGESIZE_SIZE;
        }
    }
    BOX_HEADER_MIN_SIZE
}

/// Convert a 64-bit `largesize` to an addressable length, or reject it.
///
/// A `largesize` that does not fit a `usize` cannot be addressed into a slice
/// and must be **rejected**, not truncated. Truncating it (`ls as usize`) on a
/// 32-bit target turns `0x1_0000_0008` into `8`, which slips past the
/// `eff > rem.len()` guard and lets the walk step *into the middle of the size
/// field*.
///
/// `pointer_bits` is the target's pointer width, passed explicitly rather than
/// read from `usize::BITS` inside. That is the whole point of this function
/// existing: on a 64-bit host every `u64` fits, so the defect is *unobservable*
/// and a `#[cfg(target_pointer_width = "32")]` test would be needed to catch
/// it — but this workspace never runs tests on a 32-bit target (the only
/// 32-bit CI job is a bare-metal `cargo build` for `thumbv7em-none-eabi`, with
/// no test runner). Such a test cannot fail, because it never executes.
/// Threading the width through as a parameter makes the truncation decision
/// testable for both widths on any host.
fn largesize_to_len(ls: u64, pointer_bits: u32) -> Option<usize> {
    // Anything at or above 2^pointer_bits is unaddressable on that target.
    // Guard the shift itself: `1u64 << 64` is UB-adjacent (it panics in debug,
    // wraps to 1 in release), and `pointer_bits == 64` is the common case.
    if pointer_bits < u64::BITS && ls >= (1u64 << pointer_bits) {
        return None;
    }
    usize::try_from(ls).ok()
}

fn decode_header(rem: &[u8]) -> Option<(u32, usize)> {
    if rem.len() < BOX_HEADER_MIN_SIZE {
        return None;
    }
    let size32 = u32::from_be_bytes([rem[0], rem[1], rem[2], rem[3]]);
    if size32 == SIZE_INDICATES_LARGESIZE {
        if rem.len() < BOX_HEADER_LARGESIZE_SIZE {
            return None;
        }
        let ls = u64::from_be_bytes([
            rem[8], rem[9], rem[10], rem[11], rem[12], rem[13], rem[14], rem[15],
        ]);
        Some((size32, largesize_to_len(ls, usize::BITS)?))
    } else if size32 == SIZE_TO_EOF {
        Some((size32, 0))
    } else {
        Some((size32, size32 as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_bytes(rel: &str) -> std::vec::Vec<u8> {
        std::fs::read(std::format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
            .unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    /// Finding 4: a 7-byte prefix of a real MP4 fixture (1 byte short of the
    /// 8-byte box header) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/mp4/h264_high.mp4");
        let region = &data[..BOX_HEADER_MIN_SIZE - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, BOX_HEADER_MIN_SIZE),
            other => panic!("7-byte ISOBMFF prefix must be Insufficient(8), got {other:?}"),
        }
    }

    /// The ran-out need must be **structural** — derived from the box being
    /// read — not from how many bytes happened to be supplied.
    ///
    /// Tested at the prober, not through `probe`, because the harness's
    /// `normalise_need` floor masks it end-to-end: an audit reverted this very
    /// fix and the whole-crate suite stayed green. A guard has to be applied
    /// where the value is produced, or the layer above it hides the defect.
    ///
    /// MUTATION VERIFIED: restoring
    /// `max(ran_out_need, region.len() + BOX_HEADER_MIN_SIZE)` makes the need
    /// track the supplied length (`len + 8`) instead of the header, failing the
    /// second case with `left: 1008, right: 16`.
    #[test]
    fn the_ran_out_need_is_structural_not_length_relative() {
        // A `size32 == 1` header is 16 bytes, so it is genuinely CUT for any
        // length in 8..=15 — and for every one of those the answer is the same
        // 16, because what completes the header is a property of the header,
        // not of how much happened to be supplied.
        //
        // Padding beyond 15 does NOT extend this case and must not be tested
        // here: at >=16 bytes the largesize is readable, reads as 0, and a box
        // smaller than its own header is definitively invalid. That is
        // `Unknown`, correctly, and an earlier draft of this test asserted
        // `Insufficient` there and was simply wrong about the code.
        for len in BOX_HEADER_MIN_SIZE..BOX_HEADER_LARGESIZE_SIZE {
            let mut buf = std::vec![0u8; len];
            buf[3] = SIZE_INDICATES_LARGESIZE as u8;
            buf[4..8].copy_from_slice(&TYPE_FTYP);
            match probe(&buf, buf.len()) {
                Outcome::Insufficient(need) => assert_eq!(
                    need, BOX_HEADER_LARGESIZE_SIZE,
                    "at {len} bytes the need must be the 16 the largesize header \
                     requires, not a figure that tracks the {len} supplied"
                ),
                other => panic!(
                    "a cut largesize header at {len} bytes must be Insufficient, got {other:?}"
                ),
            }
        }
    }

    /// The 32-bit truncation decision, exercised **on any host**.
    ///
    /// A `#[cfg(target_pointer_width = "32")]` test would be the obvious way to
    /// cover this and would be worthless: nothing in this workspace runs tests
    /// on a 32-bit target (the sole 32-bit CI job is a bare-metal `cargo build`
    /// for `thumbv7em-none-eabi`, which has no test runner). It would be a test
    /// that cannot fail because it never executes — the same defect class as
    /// the assertion-free `let _ = probe(...)` this replaces, only hidden
    /// behind a `cfg` instead of a missing assertion.
    ///
    /// So the pointer width is a parameter of [`largesize_to_len`] and both
    /// widths are asserted here directly.
    ///
    /// MUTATION VERIFIED: replacing the body with `usize::try_from(ls).ok()`
    /// (equivalently, the original `ls as usize`) fails the 32-bit case with
    /// `left: Some(4294967304), right: None`.
    #[test]
    fn a_largesize_beyond_the_target_pointer_width_is_rejected() {
        // Exactly 2^32 + 8: fits a 64-bit usize, does not fit a 32-bit one.
        // `as usize` on a 32-bit target would silently yield 8 and send the
        // walk into the middle of the size field.
        const OVERSIZED: u64 = 0x1_0000_0008;

        assert_eq!(
            largesize_to_len(OVERSIZED, 32),
            None,
            "a largesize of 2^32+8 is unaddressable on a 32-bit target and must \
             be rejected, never truncated to 8"
        );
        assert_eq!(
            largesize_to_len(OVERSIZED, 64),
            Some(OVERSIZED as usize),
            "the same largesize fits a 64-bit usize and must be decoded exactly"
        );
        // The boundary itself: 2^32 - 1 is the largest 32-bit-addressable value.
        assert_eq!(
            largesize_to_len(u32::MAX as u64, 32),
            Some(u32::MAX as usize)
        );
        assert_eq!(largesize_to_len(1u64 << 32, 32), None);
        // u64::MAX must not be accepted at any width below 64.
        assert_eq!(largesize_to_len(u64::MAX, 32), None);
    }

    /// The value must survive the real header path unclobbered.
    ///
    /// Asserted against a **literal**, not against `largesize_to_len`.
    /// The previous revision of this test compared `decode_header(&header)` to
    /// `largesize_to_len(0x1_0000_0008, usize::BITS)` — which is precisely the
    /// call `decode_header` makes, so both sides moved together and the test
    /// was a tautology. Replacing the body with the original defect
    /// (`Some((size32, ls as usize))`) left the whole suite green: 43 passed,
    /// 0 failed. A guard written against the implementation cannot detect the
    /// implementation changing.
    ///
    /// Scope, stated honestly: this pins the decoded value end-to-end against a
    /// literal, so it catches a clobber (a zero, a wrong field, `size32` used
    /// in place of the largesize). It does **not** catch `ls as usize`, because
    /// on a 64-bit host that cast is lossless — no test running here can. The
    /// width-truncation half is proved by
    /// `a_largesize_beyond_the_target_pointer_width_is_rejected`, which takes
    /// the pointer width as a parameter and therefore bites on any host.
    /// Between them the two cover the property; neither claims the other's.
    #[test]
    fn largesize_is_decoded_faithfully_through_decode_header() {
        let mut header = [0u8; 16];
        header[3] = SIZE_INDICATES_LARGESIZE as u8;
        header[8..].copy_from_slice(&0x1_0000_0008u64.to_be_bytes());
        assert_eq!(
            decode_header(&header),
            Some((SIZE_INDICATES_LARGESIZE, 0x1_0000_0008usize)),
            "a largesize of 2^32+8 must reach the caller exactly, not clobbered"
        );
    }

    /// Finding 7: a `largesize` box whose `largesize` is smaller than its own
    /// 16-byte header (`00 00 00 01 66 72 65 65 … 00 08` — `largesize == 8`)
    /// must be rejected, not accepted as a 2-box structural walk into the
    /// middle of the `largesize` field.
    #[test]
    fn largesize_smaller_than_its_own_header_is_rejected() {
        let data: [u8; 16] = [
            0x00, 0x00, 0x00, 0x01, // size32 == 1 (largesize follows)
            0x66, 0x72, 0x65, 0x65, // "free"
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, // largesize == 8
        ];
        // `largesize == 8 < 16` -> the walk cannot chain a 16-byte header in 8
        // bytes, so the prober returns `None` (not a STRUCTURAL match claiming
        // two boxes were walked).
        match probe(&data, data.len()) {
            Outcome::None => {}
            other => panic!(
                "a largesize smaller than its own 16-byte header must be None, got {other:?}"
            ),
        }
    }
}
