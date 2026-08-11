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
        return Outcome::None;
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

    loop {
        if offset >= region.len() {
            break;
        }
        let rem = &region[offset..];
        let (size_u32, eff) = match decode_header(rem) {
            Some(x) => x,
            None => {
                // A trailing header that cannot be read is not a clean chain.
                clean = false;
                break;
            }
        };
        // A box must at least contain its own header. A `size == 0` box runs to
        // the region end and is always valid and last. A box whose declared
        // size exceeds the remaining region is a *truncated* final box (a
        // segment clipped mid-box at the region end) — the brief's "clean
        // truncation at the region end": count it, treat the region as
        // exhausted, and keep the chain clean.
        if size_u32 == SIZE_TO_EOF || eff >= BOX_HEADER_MIN_SIZE {
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
                break;
            }
            offset += eff;
        } else {
            clean = false;
            break;
        }
    }

    if boxes == 0 {
        return Outcome::None;
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
        return Outcome::None;
    };

    Outcome::Match(Evidence { confidence, detail })
}

/// Decode a box header at the front of `rem`: the raw `u32` size and the
/// effective byte length of the box. Returns `None` when the header is
/// truncated.
///
/// The effective length is the `largesize` when `size == 1`, `0`
/// (end-of-file) when `size == 0`, else the 32-bit `size` itself.
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
        Some((size32, ls as usize))
    } else if size32 == SIZE_TO_EOF {
        Some((size32, 0))
    } else {
        Some((size32, size32 as usize))
    }
}
