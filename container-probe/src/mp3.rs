//! MPEG audio Layer III (MP3) prober — ISO/IEC 11172-3 §2.4.2.3 (frame header).
//!
//! A frame begins with an 11-bit syncword (`b[0] == 0xFF`, `(b[1] & 0xE0) ==
//! 0xE0`) whose fields encode its own `frame_length`. Detection is by **length
//! chaining**: `144 * bitrate / sample_rate + padding` must land exactly on the
//! next frame's syncword. A genuine MP3 elementary stream chains 40+ frames; a
//! container merely contains scattered `0xFF` noise (measured: every corpus
//! container's longest MP3 chain is 0-1, `audio.mp3`'s is 44).
//!
//! Only MPEG-1 Layer III is matched (`version == 3`, `layer == 1`); the
//! reserved bitrate indices (0, 15) and reserved sample-rate index (3) are
//! rejected. An ID3v2 tag at the head is skipped before anchoring.

use crate::{Confidence, Detail, Evidence, Outcome};

/// The ID3v2 tag signature (ID3v2 §3.1; the tag precedes the audio in
/// `fixtures/container-probe/audio.mp3`: `49 44 33 04`, version 2.4).
const ID3_SIGNATURE: [u8; 3] = *b"ID3";
/// Fixed portion of an ID3v2 tag header before the syncsafe size: `"ID3"`(3) +
/// version(2) + flags(1) = 6, plus the 4 syncsafe size bytes = 10.
const ID3_HEADER_LEN: usize = 10;
/// A 10-byte ID3v2 footer tag (same fixed portion plus a trailing copy) when
/// the footer flag is set (ID3v2 §3.1).
const ID3_FOOTER_LEN: usize = 10;
/// Flag bit 4 of the ID3v2 flags byte: indicates a 10-byte footer tag follows
/// (ID3v2 §3.1).
const ID3_FLAG_FOOTER: u8 = 0x10;
/// ID3v2 syncsafe size uses only the low 7 bits of each of the 4 size bytes
/// (the top bit is always 0) (ID3v2 §3.1).
const ID3_SYNCSAFE_MASK: u8 = 0x7F;
/// An MPEG audio syncword is an 11-bit `1`s pattern: ``b[0] == 0xFF`` and the
/// top 3 bits of `b[1]` set (ISO/IEC 11172-3 §2.4.2.3).
const MP3_SYNC_B1_MASK: u8 = 0xE0;
/// The MPEG version field `b[1][4:3]`: `3` is MPEG-1 (`11`).
const MP3_VERSION_MPEG1: u8 = 3;
/// The MPEG layer field `b[1][2:1]`: `1` is Layer III (`01`) (transmux
/// `mpeg_legacy.rs`: `LAYER_III = 0b01`).
const MP3_LAYER_III: u8 = 1;
/// `frame_length` = `FRAME_LENGTH_COEFF * bitrate / sample_rate + padding`
/// (ISO/IEC 11172-3 §2.4.2.3; `samples_per_frame/8` = 1152/8 = 144).
const FRAME_LENGTH_COEFF: u32 = 144;
/// Minimum valid MP3 frame length (the 4-byte header; real frames are larger).
const MP3_MIN_FRAME_LEN: usize = 4;
/// The bytes a 4-byte MP3 frame header occupies; a region shorter than this
/// cannot offer one header, so the prober cannot rule MP3 out.
const MP3_HEADER_LEN: usize = 4;
/// Convert the kbps bitrate table to bits per second for `frame_length`.
const BITRATE_KILO_FACTOR: u32 = 1000;
/// Minimum chained frames for a positive `LATTICE_WEAK` match.
const MP3_MIN_CHAIN_WEAK: usize = 4;
/// Chained frames that lift the verdict to `LATTICE_STRONG`.
const MP3_MIN_CHAIN_STRONG: usize = 16;

/// Bitrate table (kbps) for MPEG-1 Layer III by `bitrate_index`
/// (`b[2][7:4]`); index 0 (free-format) and 15 (forbidden) are `0` and
/// rejected (ISO/IEC 11172-3 §2.4.2.3; transmux `mpeg_legacy.rs`).
const BITRATE_KBPS: [u16; 16] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
];
/// Sample rates (Hz) for MPEG-1 by `sample_rate_index` (`b[2][3:2]`); index 3
/// is reserved (ISO/IEC 11172-3 §2.4.2.3; transmux `mpeg_legacy.rs`).
const SAMPLE_RATE_MPEG1: [u32; 3] = [44_100, 48_000, 32_000];

/// The registered MP3 prober: ID3 skip + longest length-chained frame run.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    // Shorter than one 4-byte frame header: undecided (`Insufficient`), not
    // `Unknown` — a truncated .mp3 read a few bytes at a time could still be a
    // frame's syncword.
    if region.len() < MP3_HEADER_LEN {
        return Outcome::Insufficient(MP3_HEADER_LEN);
    }

    let (longest, truncated, anchor, frame_len) = longest_mp3_chain(region);

    if longest >= MP3_MIN_CHAIN_STRONG {
        return Outcome::Match(Evidence {
            confidence: Confidence::LATTICE_STRONG,
            detail: Detail::None,
        });
    }
    if longest >= MP3_MIN_CHAIN_WEAK {
        return Outcome::Match(Evidence {
            confidence: Confidence::LATTICE_WEAK,
            detail: Detail::None,
        });
    }
    if longest == 0 {
        // No valid MP3 frame header anywhere -> nothing to build on.
        return Outcome::None;
    }
    // A partial chain of 1..=(WEAK-1) frames: `Insufficient` only when it ended
    // by running off the end of the buffer (`truncated`); a chain that ended at
    // an invalid next header is a scattered frame and will not become an MP3
    // stream by reading further. Mirrors the TS prober's `could_prove`.
    if truncated {
        Outcome::Insufficient(need_at_least(anchor, frame_len, region.len()))
    } else {
        Outcome::None
    }
}

/// A lower bound on bytes that could resolve the verdict to `LATTICE_WEAK`:
/// room at the observed frame size for [`MP3_MIN_CHAIN_WEAK`] frames from the
/// chain's anchor, and strictly more than the caller already holds.
fn need_at_least(anchor: usize, frame_len: usize, have: usize) -> usize {
    core::cmp::max(
        anchor.saturating_add(MP3_MIN_CHAIN_WEAK.saturating_mul(frame_len)),
        have.saturating_add(frame_len),
    )
}

/// The longest chain of valid MPEG-1 Layer III frames linked by their own
/// `frame_length` fields, scanning from every candidate position (after
/// skipping any leading ID3v2 tag), together with whether that chain ended by
/// running off the buffer (`truncated`) or hitting an invalid next header, plus
/// its anchor offset and first-frame length. Short-circuits at
/// [`MP3_MIN_CHAIN_STRONG`], bounding the work.
fn longest_mp3_chain(data: &[u8]) -> (usize, bool, usize, usize) {
    let n = data.len();
    let mut best = 0usize;
    let mut best_truncated = false;
    let mut best_anchor = 0usize;
    let mut best_frame_len = 0usize;
    let mut i = id3_skip(data).unwrap_or(0);
    while i < n {
        if let Some(first_len) = mp3_frame_len(data, i) {
            let mut p = i;
            let mut run = 0usize;
            let mut truncated = false;
            while let Some(l) = mp3_frame_len(data, p) {
                run += 1;
                if run >= MP3_MIN_CHAIN_STRONG {
                    return (run, false, i, first_len);
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

/// `frame_length` for a valid MPEG-1 Layer III frame header at `i`, or `None`
/// if the bytes are not a conformant MP3 frame header.
fn mp3_frame_len(data: &[u8], i: usize) -> Option<usize> {
    if i + 3 >= data.len() {
        return None;
    }
    let b0 = data[i];
    let b1 = data[i + 1];
    let b2 = data[i + 2];
    let b3 = data[i + 3];
    if b0 != 0xFF || b1 & MP3_SYNC_B1_MASK != MP3_SYNC_B1_MASK {
        return None;
    }
    let version = (b1 >> 3) & 0x03;
    let layer = (b1 >> 1) & 0x03;
    if version != MP3_VERSION_MPEG1 || layer != MP3_LAYER_III {
        return None;
    }
    let br_idx = (b2 >> 4) & 0x0F;
    let sr_idx = (b2 >> 2) & 0x03;
    if usize::from(sr_idx) >= SAMPLE_RATE_MPEG1.len() {
        return None;
    }
    let bitrate = BITRATE_KBPS[br_idx as usize];
    if bitrate == 0 {
        return None; // reserved/free-format
    }
    let padding = (b3 >> 1) & 0x01;
    let len = (FRAME_LENGTH_COEFF * u32::from(bitrate) * BITRATE_KILO_FACTOR)
        / SAMPLE_RATE_MPEG1[sr_idx as usize]
        + u32::from(padding);
    let len = len as usize;
    if len < MP3_MIN_FRAME_LEN {
        return None;
    }
    Some(len)
}

/// The number of bytes to skip past a leading ID3v2 tag, or `None` when there
/// is none (ID3v2 §3.1). A footer, when flagged, adds 10 bytes.
fn id3_skip(data: &[u8]) -> Option<usize> {
    if data.len() < ID3_HEADER_LEN || data[..ID3_SIGNATURE.len()] != ID3_SIGNATURE {
        return None;
    }
    let flags = data[5];
    let size = ((u32::from(data[6]) & u32::from(ID3_SYNCSAFE_MASK)) << 21)
        | ((u32::from(data[7]) & u32::from(ID3_SYNCSAFE_MASK)) << 14)
        | ((u32::from(data[8]) & u32::from(ID3_SYNCSAFE_MASK)) << 7)
        | (u32::from(data[9]) & u32::from(ID3_SYNCSAFE_MASK));
    let footer = if flags & ID3_FLAG_FOOTER != 0 {
        ID3_FOOTER_LEN as u32
    } else {
        0
    };
    Some(ID3_HEADER_LEN + size as usize + footer as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_bytes(rel: &str) -> std::vec::Vec<u8> {
        std::fs::read(std::format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
            .unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    /// The chain-threshold discriminator (mutation #2 for the MP3 chain rule).
    ///
    /// A genuine container file carries a single valid MPEG-1 Layer III frame
    /// somewhere in its payload (longest MP3 chain of 1), so with the real
    /// `MP3_MIN_CHAIN_WEAK = 4` the MP3 prober returns `None`. Dropping the
    /// threshold to 1 (raw frame counting) misidentifies the same file as Mp3.
    /// Observed under the mutation (`MP3_MIN_CHAIN_WEAK = 1`):
    /// ```
    /// h264_aac.ts must NOT match MP3 at the real threshold,
    ///   got Match(Evidence { confidence: Confidence(96), detail: None })
    /// ```
    #[test]
    fn chain_threshold_keeps_a_container_out() {
        let data = fixture_bytes("fixtures/ts/h264_aac.ts");
        match probe(&data, data.len()) {
            Outcome::None => {}
            other => panic!("h264_aac.ts must NOT match MP3 at the real threshold, got {other:?}"),
        }
    }

    /// ID3v2-skip guard (mutation #4). The real fixture `audio.mp3` is
    /// ID3v2-prefixed: `id3_skip` must return 45 and a valid MP3 frame header
    /// must sit there, so the prober sees the audio.
    ///
    /// Note the honest limitation this isolation test guards: because the MP3
    /// chain search is *position-independent*, nulling the ID3 skip (making
    /// `id3_skip` return 0) does NOT change the final verdict on `audio.mp3` —
    /// the search still reaches the frames at offset 45. The naive
    /// "remove skip -> no longer Mp3" expectation does not hold, so this test
    /// isolates the skip's correctness directly. Observed under the mutation
    /// (`id3_skip` forced to `Some(0)`):
    /// ```
    /// assertion `left == right` failed
    ///   left: 0
    /// right: 45
    /// ```
    #[test]
    fn audio_mp3_id3_skip_is_correct() {
        let data = fixture_bytes("fixtures/container-probe/audio.mp3");
        let skip = id3_skip(&data).expect("audio.mp3 is ID3-prefixed");
        assert_eq!(skip, 45);
        assert!(
            mp3_frame_len(&data, skip).is_some(),
            "a frame must follow the tag"
        );
        // The real fixture must identify as Mp3 at the LATTICE_STRONG tier.
        match probe(&data, data.len()) {
            Outcome::Match(ev) => {
                assert_eq!(ev.confidence, Confidence::LATTICE_STRONG);
            }
            other => panic!("audio.mp3 must identify as Mp3, got {other:?}"),
        }
    }

    /// Finding 4: a 3-byte prefix of a real MP3 fixture (1 byte short of the
    /// 4-byte frame header) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/container-probe/audio.mp3");
        let region = &data[..MP3_HEADER_LEN - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, MP3_HEADER_LEN),
            other => panic!("3-byte MP3 prefix must be Insufficient(4), got {other:?}"),
        }
    }
}
