//! AAC ADTS prober — ISO/IEC 13818-7 §6.2 (Audio Data Transport Stream).
//!
//! A frame starts with a 12-bit syncword `0xFFF` with `layer == 00` (the brief
//! states the second byte masked with `0xF6` compares to `0xF0`), and its
//! 13-bit `frame_length` (**bytes 3-5**) points exactly to the next frame's
//! syncword. Detection is by **length chaining**, not raw syncword counting:
//! a genuine AAC elementary stream chains 40+ frames, while a container merely
//! contains scattered `0xFFF` noise (measured: every corpus container's longest
//! ADTS chain is 0-1, `aac.adts`'s is 48).
//!
//! The longest chain of valid frames linked by their own `frame_length` fields:
//! `>= 16` -> `LATTICE_STRONG`, `>= 4` -> `LATTICE_WEAK`, fewer -> no match.

use crate::{Confidence, Detail, Evidence, Outcome};

/// A conformant ADTS profile/layer byte masked to its `layer` + `profile`
/// bits: `0xFF` sync hi, `0xF6` selects layer/profile (ISO/IEC 13818-7 §6.2.1;
/// `layer` must be `00`), compared to `0xF0`.
const ADTS_LAYER_MASK: u8 = 0xF6;
/// The ADTS syncword bytes (`0xFF` + `0xF0` profile/layer pattern).
const ADTS_SYNC: [u8; 2] = [0xFF, 0xF0];
/// Bits 1:0 of byte 3 and the whole of bytes 4-5 carry the 13-bit
/// `frame_length` (ISO/IEC 13818-7 §6.2.1).
const ADTS_FRAME_LENGTH_SHIFT: u8 = 3;
/// Minimum aac profile frame length for a plausible frame (a header alone is
/// 7 bytes; shorter is not a real ADTS frame).
const ADTS_MIN_FRAME_LEN: usize = 7;
/// Minimum valid-frames-in-a-chain for a positive `LATTICE_WEAK` match.
const ADTS_MIN_CHAIN_WEAK: usize = 4;
/// Valid-frames-in-a-chain that lift the verdict to `LATTICE_STRONG`.
const ADTS_MIN_CHAIN_STRONG: usize = 16;

/// The registered ADTS prober: longest length-chained frame run over `limit`.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    let longest = longest_adts_chain(region);

    if longest >= ADTS_MIN_CHAIN_STRONG {
        return Outcome::Match(Evidence {
            confidence: Confidence::LATTICE_STRONG,
            detail: Detail::None,
        });
    }
    if longest >= ADTS_MIN_CHAIN_WEAK {
        return Outcome::Match(Evidence {
            confidence: Confidence::LATTICE_WEAK,
            detail: Detail::None,
        });
    }
    Outcome::None
}

/// `true` if a valid ADTS frame header sits at `i` (with room for its body).
fn adts_frame_len(data: &[u8], i: usize) -> Option<usize> {
    if i + 6 > data.len() {
        return None;
    }
    if data[i] != ADTS_SYNC[0] || data[i + 1] & ADTS_LAYER_MASK != ADTS_SYNC[1] {
        return None;
    }
    let frame_len = ((usize::from(data[i + 3]) & 0x03) << ADTS_FRAME_LENGTH_SHIFT)
        | (usize::from(data[i + 4]) << 3)
        | (usize::from(data[i + 5]) >> 5);
    if frame_len < ADTS_MIN_FRAME_LEN {
        return None;
    }
    Some(frame_len)
}

/// The length of the longest chain of valid ADTS frames linked by their own
/// `frame_length` fields anywhere in `data`. Bounded: a chain that reaches
/// [`ADTS_MIN_CHAIN_STRONG`] short-circuits to that value, so no buffer forces
/// more than a bounded number of link steps.
fn longest_adts_chain(data: &[u8]) -> usize {
    let mut best = 0usize;
    let n = data.len();
    let mut i = 0usize;
    while i < n {
        if adts_frame_len(data, i).is_some() {
            // Count the chain anchored at `i`.
            let mut p = i;
            let mut run = 0usize;
            while let Some(l) = adts_frame_len(data, p) {
                run += 1;
                if run >= ADTS_MIN_CHAIN_STRONG {
                    return run; // strong reached; no need to measure further
                }
                p += l;
                if p <= i {
                    break; // zero/non-forward length guard
                }
            }
            best = best.max(run);
            i += 1;
        } else {
            i += 1;
        }
    }
    best
}

/// Count the number of valid ADTS frame headers anywhere in `data` — used by
/// the in-module suppression test to show a buffer's payload carries ADTS
/// frames even when none form a long enough chain to match.
#[cfg(test)]
fn synthetic_ts_carrying_adts(frame_count: usize) -> alloc::vec::Vec<u8> {
    let aac_len = frame_count * 274usize;
    // The TS prefix must dominate the buffer so the TS lattice keeps >= 50%
    // coverage over the whole buffer (the harness suppression test relies on
    // BOTH probers firing). Use enough 188-byte packets that the prefix is at
    // least as large as the appended ADTS region, and never fewer than 12.
    let packets = core::cmp::max(12, aac_len.div_ceil(188) * 2);
    let mut v = alloc::vec::Vec::new();
    for _ in 0..packets {
        let mut pkt = alloc::vec![0u8; 188];
        pkt[0] = 0x47;
        v.extend_from_slice(&pkt);
    }
    // Append `frame_count` ADTS frames chained by their length fields.
    let frame_len: u16 = 274;
    for _ in 0..frame_count {
        let mut f = alloc::vec![0u8; frame_len as usize];
        f[0] = 0xFF;
        f[1] = 0xF1; // layer 00, profile 1
        f[3] = ((frame_len >> 11) & 0x03) as u8;
        f[4] = ((frame_len >> 3) & 0xFF) as u8;
        f[5] = ((frame_len & 0x07) as u8) << 5;
        v.extend_from_slice(&f);
    }
    v
}
#[cfg(test)]
pub(crate) fn adts_frame_count(data: &[u8]) -> usize {
    let mut c = 0usize;
    let mut i = 0usize;
    while i + 6 <= data.len() {
        if adts_frame_len(data, i).is_some() {
            c += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a workspace-relative fixture (crate is `no_std`; the test build
    /// links `std`).
    fn fixture_bytes(rel: &str) -> std::vec::Vec<u8> {
        std::fs::read(std::format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
            .unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    /// The suppression half of the "an ES prober fires on a container's
    /// payload" proof (WP3): a synthetic buffer that is BOTH a genuine TS
    /// lattice AND a >=16-frame ADTS chain. It is synthetic - it exists to
    /// exercise the harness's cross-prober suppression rule, not to claim any
    /// real file does this (no corpus container has an ADTS chain > 1).
    ///
    /// Without suppression this probes Ambiguous (MpegTs 144 vs AdtsAac 144);
    /// with it, MpegTs. The assembly-level assertion lives in
    /// `tests/fixture_magic_es.rs`; here we confirm the ADTS prober itself
    /// fires directly (chain >= 16) on the TS payload.
    #[test]
    fn adts_prober_fires_on_ts_carrying_adts() {
        let data = synthetic_ts_carrying_adts(32);
        // The TS prober must also see this as a strong TS lattice.
        let ts = crate::probe_with_budget(&data, data.len());
        assert!(matches!(
            ts,
            crate::Probe::Identified {
                format: crate::Format::MpegTs,
                ..
            }
        ));

        // The TS payload genuinely carries ADTS frames (raw count > 0).
        assert!(adts_frame_count(&data) > 0);

        // The ADTS prober alone fires at LATTICE_STRONG on the same bytes.
        match probe(&data, data.len()) {
            Outcome::Match(ev) => {
                assert_eq!(ev.confidence, Confidence::LATTICE_STRONG);
            }
            other => panic!("adts probe must fire on the payload, got {other:?}"),
        }
    }

    /// The chain-threshold discriminator (mutation #1 for the ADTS chain rule).
    ///
    /// A genuine TS file carries a *single* valid ADTS frame somewhere in its
    /// payload (the model counts a longest ADTS chain of 1), so with the real
    /// `ADTS_MIN_CHAIN_WEAK = 4` the ADTS prober returns `None`. If that
    /// threshold is dropped to 1 (raw frame counting), the same file is
    /// misidentified — the false positive the chaining rule exists to prevent.
    /// Observed under the mutation (`ADTS_MIN_CHAIN_WEAK = 1`):
    /// ```
    /// h264_aac.ts must NOT match ADTS at the real threshold,
    ///   got Match(Evidence { confidence: Confidence(96), detail: None })
    /// ```
    #[test]
    fn chain_threshold_keeps_a_container_out() {
        let data = fixture_bytes("fixtures/ts/h264_aac.ts");
        assert!(
            adts_frame_count(&data) > 0,
            "container must carry ADTS frames"
        );
        match probe(&data, data.len()) {
            Outcome::None => {}
            other => panic!("h264_aac.ts must NOT match ADTS at the real threshold, got {other:?}"),
        }
    }
}
