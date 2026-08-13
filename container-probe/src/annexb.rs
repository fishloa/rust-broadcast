//! H.264 Annex B prober — ITU-T H.264 §7.4 / Annex B byte-stream NAL unit
//! format (start codes `00 00 01` / `00 00 00 01`).
//!
//! **Annex B detection is H.264 only.** HEVC (H.265) is NOT detected, and no
//! claim is made that it is: HEVC uses a 2-byte NAL header with
//! `nal_unit_type` at bits `[6:1]`, not the H.264 1-byte header masked with
//! `0x1F` that this prober validates, so an HEVC stream fails the range check
//! and its chain breaks at the first NAL. This workspace does not implement a
//! format without a real fixture to test it against, and there is no HEVC
//! Annex B fixture in the repo — see the crate-root "Known gaps". The
//! `AnnexB` format name refers to the framing, not to both codecs.
//!
//! Requires a start code **at offset 0** and validates each NAL unit header:
//! `forbidden_zero_bit` (the top bit) MUST be clear and `nal_unit_type` in a
//! plausible range. It then advances to the next start code and validates that
//! header too. `>= 16` consecutive valid NAL headers -> `LATTICE_STRONG`,
//! `>= 4` -> `LATTICE_WEAK`, fewer -> no match.
//!
//! Validating the NAL header is what excludes MPEG-PS: `fixtures/ps/h264_ac3.ps`
//! begins `00 00 01 BA`, where `0xBA` has `forbidden_zero_bit` SET (`0x80`),
//! which is illegal — so it is not Annex B, while `h264.annexb`'s first NAL
//! byte `0x67` (type 7 SPS) passes.

use crate::{Confidence, Detail, Evidence, Outcome};

/// The 3-byte start-code prefix `00 00 01` (ITU-T H.264 §7.4.1).
const START_CODE_3: [u8; 3] = [0x00, 0x00, 0x01];
/// The 4-byte start-code `00 00 00 01` (ITU-T H.264 Annex B).
const START_CODE_4: [u8; 4] = [0x00, 0x00, 0x00, 0x01];
/// `forbidden_zero_bit` must be **`0`** (clear) in a conformant H.264 NAL header
/// (ITU-T H.264 §7.4.1); this mask is what rejects MPEG-PS's `00 00 01 BA`.
const NAL_FORBIDDEN_ZERO: u8 = 0x80;
/// Mask to extract `nal_unit_type` (low 5 bits of the header byte).
const NAL_TYPE_MASK: u8 = 0x1F;
/// Lowest `nal_unit_type` that is a plain NAL unit (1 = non-IDR slice, H.264;
/// 0 is reserved/unspecified). Types 24-31 are RTP aggregation/filler forms
/// with their own 3-byte headers and do not occur as single-header byte-stream
/// NALs; they are accepted here at probe granularity because the
/// `forbidden_zero_bit` check, not the type value, is the discriminator that
/// excludes non-H.264 data (e.g. MPEG-PS's `00 00 01 BA`).
const NAL_TYPE_MIN: u8 = 1;
/// Highest `nal_unit_type` value (5-bit mask upper bound, 31).
const NAL_TYPE_MAX: u8 = 0x1F;
/// Minimum chained NAL headers for a positive `LATTICE_WEAK` match.
const ANNEXB_MIN_NALS_WEAK: usize = 4;
/// Chained NAL headers that lift the verdict to `LATTICE_STRONG`.
const ANNEXB_MIN_NALS_STRONG: usize = 16;
/// Minimum bytes to attempt the first NAL: the shortest start code (3 bytes)
/// plus the 1-byte NAL header that follows it. A shorter region cannot even
/// begin validating a NAL unit, so the prober cannot rule Annex B out.
const ANNEXB_MIN_LEN: usize = START_CODE_3.len() + 1;

/// The registered Annex B prober: start code at offset 0 + chained NAL headers.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    // Shorter than one start code + NAL header: undecided (`Insufficient`), not
    // `Unknown` — a truncated .h264 read a few bytes at a time could still be
    // the start codes this prober looks for.
    if region.len() < ANNEXB_MIN_LEN {
        return Outcome::Insufficient(ANNEXB_MIN_LEN);
    }

    let (chain, truncated) = annexb_nal_chain(region);

    if chain >= ANNEXB_MIN_NALS_STRONG {
        return Outcome::Match(Evidence {
            confidence: Confidence::LATTICE_STRONG,
            detail: Detail::None,
        });
    }
    if chain >= ANNEXB_MIN_NALS_WEAK {
        return Outcome::Match(Evidence {
            confidence: Confidence::LATTICE_WEAK,
            detail: Detail::None,
        });
    }
    if chain == 0 {
        // No valid NAL header (either no start code at offset 0, or the first
        // NAL header was invalid) -> nothing to build on.
        return Outcome::None;
    }
    // A partial chain of 1..=(WEAK-1) NAL units. `Insufficient` only when the
    // chain ended because the buffer ran out mid-stream (`truncated`) — a
    // longer buffer could still confirm more NALs. When the chain ended because
    // a NAL header failed to validate within the region, more bytes will not
    // change that: the start codes were payload noise.
    if truncated {
        Outcome::Insufficient(need_at_least(region.len()))
    } else {
        Outcome::None
    }
}

/// A lower bound on bytes that could resolve the verdict to `LATTICE_WEAK`: at
/// least [`ANNEXB_MIN_NALS_WEAK`] minimal NAL units, and strictly more than the
/// caller already holds (so "supply more" always exceeds what was supplied).
fn need_at_least(have: usize) -> usize {
    core::cmp::max(ANNEXB_MIN_NALS_WEAK * ANNEXB_MIN_LEN, have + ANNEXB_MIN_LEN)
}

/// Length (in bytes) of the start code at `i`, or `0` if none. A 4-byte start
/// code is also a 3-byte one, so we try the 4-byte form first.
fn start_code_len(data: &[u8], i: usize) -> usize {
    if start_code4_at(data, i) {
        START_CODE_4.len()
    } else if start_code3_at(data, i) {
        START_CODE_3.len()
    } else {
        0
    }
}

fn start_code4_at(data: &[u8], i: usize) -> bool {
    i + START_CODE_4.len() <= data.len()
        && data.get(i..i + START_CODE_4.len()) == Some(&START_CODE_4[..])
}

fn start_code3_at(data: &[u8], i: usize) -> bool {
    i + START_CODE_3.len() <= data.len()
        && data.get(i..i + START_CODE_3.len()) == Some(&START_CODE_3[..])
}

/// `true` when `data[i..]` begins a start code whose NAL header byte is valid.
fn valid_nal(data: &[u8], i: usize) -> bool {
    let sc = start_code_len(data, i);
    if sc == 0 {
        return false;
    }
    let Some(&header) = data.get(i + sc) else {
        return false;
    };
    // forbidden_zero_bit must be clear; nar_ref_idc is 2 bits (ignored);
    // nal_unit_type must be a plausible non-aggregation value.
    if header & NAL_FORBIDDEN_ZERO != 0 {
        return false;
    }
    let nal_type = header & NAL_TYPE_MASK;
    (NAL_TYPE_MIN..=NAL_TYPE_MAX).contains(&nal_type)
}

/// Walk the Annex B byte stream from offset 0 counting consecutive NAL units,
/// each terminated by a start code whose NAL header validates. Non-conformant
/// NAL headers or gaps stop the count.
///
/// Returns the count and whether the walk stopped because the buffer ran out
/// mid-stream (`truncated == true`) rather than because a NAL header failed to
/// validate (`truncated == false`). The distinction is what decides
/// `Insufficient` ("read more") vs `None` ("stop") for a partial chain.
fn annexb_nal_chain(data: &[u8]) -> (usize, bool) {
    let n = data.len();
    // A start code must be at offset 0 — Annex B begins with the byte stream's
    // first NAL unit boundary, never with payload mid-stream.
    if start_code_len(data, 0) == 0 {
        return (0, false);
    }
    let mut cnt = 0usize;
    let mut i = 0usize;
    loop {
        let sc = start_code_len(data, i);
        if sc == 0 {
            // The previous advance left `i` pointing past the buffer's start
            // codes — the stream was cut off before a terminating start code.
            return (cnt, true);
        }
        // Validate the NAL header immediately after this start code.
        if !valid_nal(data, i) {
            return (cnt, false);
        }
        cnt += 1;
        if cnt >= ANNEXB_MIN_NALS_STRONG {
            return (cnt, false);
        }
        // Advance to the next start code strictly after the current header
        // byte; the first plausible start-code location is the unit boundary.
        let mut next = i + sc + 1;
        while next + 2 <= n && start_code_len(data, next) == 0 {
            next += 1;
        }
        if next + 2 > n {
            // Fewer than three bytes remain: no terminating start code is
            // visible, so the buffer ended mid-NAL.
            return (cnt, true);
        }
        i = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_bytes(rel: &str) -> std::vec::Vec<u8> {
        std::fs::read(std::format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
            .unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    /// The `forbidden_zero_bit` discriminator (mutation #3). MPEG-PS begins
    /// `00 00 01 BA`: a start code, but the NAL header `0xBA` has
    /// `forbidden_zero_bit` SET, so Annex B must reject it. Removing that check
    /// lets the Annex B walker accept `0xBA` (and the stream's other start-code
    /// NALs), misidentifying the PS as Annex B. Observed under the mutation:
    /// ```
    /// h264_ac3.ps must NOT match AnnexB (forbidden bit),
    ///   got Match(Evidence { confidence: Confidence(144), detail: None })
    /// ```
    #[test]
    fn forbidden_zero_bit_keeps_ps_out_of_annexb() {
        let data = fixture_bytes("fixtures/ps/h264_ac3.ps");
        match probe(&data, data.len()) {
            Outcome::None => {}
            other => panic!("h264_ac3.ps must NOT match AnnexB (forbidden bit), got {other:?}"),
        }
    }

    /// Finding 4: a 3-byte prefix of a real H.264 Annex B fixture (1 byte short
    /// of a start code + NAL header) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/container-probe/h264.annexb");
        let region = &data[..ANNEXB_MIN_LEN - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, ANNEXB_MIN_LEN),
            other => panic!("3-byte Annex B prefix must be Insufficient(4), got {other:?}"),
        }
    }
}
