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
/// `frame_length` (ISO/IEC 13818-7 §6.2.1): `byte3[1:0]` are bits `[12:11]`,
/// `byte4` is `[10:3]`, `byte5[7:5]` is `[2:0]`. So the two high bits from byte 3
/// shift left by **11** (they land at bit 12 and 11 of the 13-bit field).
/// `frame_len = ((byte3 & 0x03) << 11) | (byte4 << 3) | (byte5 >> 5)`.
const ADTS_FRAME_LENGTH_SHIFT: u8 = 11;
/// Minimum aac profile frame length for a plausible frame (a header alone is
/// 7 bytes; shorter is not a real ADTS frame).
const ADTS_MIN_FRAME_LEN: usize = 7;
/// The number of header bytes `adts_frame_len` reads: the 12-bit syncword +
/// layer byte + the three frame-length bytes (through byte 5). A region shorter
/// than this cannot even offer one header, so the prober cannot rule ADTS out.
const ADTS_HEADER_LEN: usize = 6;
/// Minimum valid-frames-in-a-chain for a positive `LATTICE_WEAK` match.
const ADTS_MIN_CHAIN_WEAK: usize = 4;
/// Valid-frames-in-a-chain that lift the verdict to `LATTICE_STRONG`.
const ADTS_MIN_CHAIN_STRONG: usize = 16;

/// The registered ADTS prober: longest length-chained frame run over `limit`.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    // Shorter than one full header: the prober cannot even read a `frame_length`
    // to start a chain, so a truncated .aac read a few bytes at a time is
    // undecided (`Insufficient`), not `Unknown`.
    if region.len() < ADTS_HEADER_LEN {
        return Outcome::Insufficient(ADTS_HEADER_LEN);
    }

    let (longest, truncated, anchor, frame_len) = longest_adts_chain(region);

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
    if longest == 0 {
        // No valid ADTS frame header anywhere in the region — nothing to build
        // on, and more bytes will not help.
        return Outcome::None;
    }
    // A non-empty chain short of the weak threshold: a partial chain of
    // 1..=(WEAK-1) valid frames. `Insufficient` only when the chain ended
    // because the buffer ran out mid-frame (`truncated`) — a longer buffer
    // could still confirm it (a truncated stream read a few bytes at a time).
    // When it ended because the next header failed to validate within the
    // region, more bytes will not change that: the frame is a scattered
    // syncword inside some other payload. This mirrors the TS prober's
    // `could_prove` distinction.
    if truncated {
        Outcome::Insufficient(need_at_least(anchor, frame_len, region.len()))
    } else {
        Outcome::None
    }
}

/// A lower bound on bytes that could resolve the verdict to `LATTICE_WEAK`:
/// enough room at the observed frame size for [`ADTS_MIN_CHAIN_WEAK`] frames
/// from the chain's anchor, and strictly more than the caller already holds
/// (so "supply more" always exceeds what was supplied).
fn need_at_least(anchor: usize, frame_len: usize, have: usize) -> usize {
    core::cmp::max(
        anchor.saturating_add(ADTS_MIN_CHAIN_WEAK.saturating_mul(frame_len)),
        have.saturating_add(frame_len),
    )
}

/// `true` if a valid ADTS frame header sits at `i`, and if so the
/// 13-bit `frame_length`. Only the 6 bytes through byte 5 (the syncword, the
/// layer byte and the three length bytes) are read — the frame's *body* is
/// deliberately not inspected, so the check is "a header fits", not "the whole
/// frame fits".
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

/// The longest chain of valid ADTS frames linked by their own `frame_length`
/// fields anywhere in `data`, together with whether that chain ended by running
/// off the end of the buffer (`truncated`) rather than hitting an invalid next
/// header, plus the chain's anchor offset and first-frame length (used to size
/// the `Insufficient` hint). Bounded: a chain that reaches
/// [`ADTS_MIN_CHAIN_STRONG`] short-circuits to that value, so no buffer forces
/// more than a bounded number of link steps.
fn longest_adts_chain(data: &[u8]) -> (usize, bool, usize, usize) {
    let mut best = 0usize;
    let mut best_truncated = false;
    let mut best_anchor = 0usize;
    let mut best_frame_len = 0usize;
    let n = data.len();
    let mut i = 0usize;
    while i < n {
        if let Some(first_len) = adts_frame_len(data, i) {
            // Count the chain anchored at `i`.
            let mut p = i;
            let mut run = 0usize;
            let mut truncated = false;
            while let Some(l) = adts_frame_len(data, p) {
                run += 1;
                if run >= ADTS_MIN_CHAIN_STRONG {
                    return (run, false, i, first_len); // strong reached; stop early
                }
                if p + l > n {
                    // The frame's body extends past the buffer: a genuine
                    // truncation (the stream was cut mid-frame).
                    truncated = true;
                    break;
                }
                p += l;
            }
            if run > best || (run == best && truncated) {
                best = run;
                best_truncated = truncated;
                best_anchor = i;
                best_frame_len = first_len;
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    (best, best_truncated, best_anchor, best_frame_len)
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

    /// Pins the synthetic TS+ADTS byte layout so the duplicated builder in
    /// `tests/fixture_magic_es.rs` (WP4 of the "remove duplication" task) cannot
    /// silently drift from this crate-internal one.
    ///
    /// The integration test cannot see the `#[cfg(test)]` helper here (it is
    /// external to the crate), so no direction removes the duplication without
    /// widening a `pub(crate)` item. Both are left in place; this assertion is
    /// the drift tripwire — it fixes the exact layout the two builders must
    /// agree on, so a divergence becomes a test failure rather than a quiet
    /// change in whether one suppression proof fires.
    #[test]
    fn synthetic_ts_carrying_adts_layout_is_pinned() {
        let data = synthetic_ts_carrying_adts(32);
        // 94 whole 188-byte TS packets, then 32 274-byte ADTS frames.
        let base = 94 * 188;
        assert_eq!(data.len(), base + 32 * 274);
        // Every packet starts with the TS sync byte.
        for p in 0..94 {
            assert_eq!(
                data[p * 188],
                0x47,
                "packet {p} must start with the sync byte"
            );
        }
        // The ADTS region begins after the TS prefix with a valid chained frame.
        assert_eq!(data[base], 0xFF);
        assert_eq!(data[base + 1], 0xF1);
        // frame_length = 274, encoded across bytes 3-5.
        assert_eq!(data[base + 3], 0);
        assert_eq!(data[base + 4], (274u16 >> 3) as u8);
        assert_eq!(data[base + 5], ((274u16 & 0x07) as u8) << 5);
        // Last ADTS frame present and self-consistent.
        let last = base + 31 * 274;
        assert_eq!(data[last], 0xFF);
        assert_eq!(data[last + 4], (274u16 >> 3) as u8);
    }

    /// Finding 2: a frame >= 2048 bytes round-trips exactly.
    ///
    /// The prior `ADTS_FRAME_LENGTH_SHIFT` was 3 instead of 11, so every
    /// `frame_length >= 0x800` decoded wrong (e.g. 2500 decoded as 460) and the
    /// length chain broke, mis-chaining every real high-bitrate frame. The
    /// fixture fixtures used `frame_len = 274`, and `274 >> 11 == 0`, so the
    /// high two bits of the 13-bit field were never exercised — a 274-byte frame
    /// cannot prove the top of the field is decoded correctly. Here we encode a
    /// `frame_len` that *needs* those two high bits and assert it decodes back
    /// byte-exactly.
    #[test]
    fn large_frame_length_round_trips() {
        // Encode `frame_len` the same way the fixtures do (ISO/IEC 13818-7
        // §6.2.1): bit [12:11] in byte 3 [1:0], [10:3] in byte 4, [2:0] in
        // byte 5 [7:5].
        let encode = |frame_len: u16| -> [u8; 6] {
            [
                0xFF, // sync hi
                0xF1, // layer 00, profile 1
                0x00, // not read
                ((frame_len >> 11) & 0x03) as u8,
                ((frame_len >> 3) & 0xFF) as u8,
                (((frame_len & 0x07) as u8) << 5),
            ]
        };
        // 2050 needs both high bits: 2050 = 0b1000_0000_0010, bit 11 set.
        for frame_len in [0x800u16, 2050, 0x1FFF] {
            let hdr = encode(frame_len);
            let decoded = adts_frame_len(&hdr, 0).expect("valid header must decode");
            assert_eq!(
                usize::from(frame_len),
                decoded,
                "13-bit frame_length {frame_len} must round-trip exactly"
            );
        }
    }

    /// Finding 4: a 5-byte prefix of a real ADTS fixture (1 byte short of the
    /// 6-byte header the length decoder needs) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/container-probe/aac.adts");
        // 6 header bytes are the minimum the decoder reads; 5 is 1 short.
        let region = &data[..ADTS_HEADER_LEN - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, ADTS_HEADER_LEN),
            other => panic!("5-byte ADTS prefix must be Insufficient(6), got {other:?}"),
        }
    }
}
