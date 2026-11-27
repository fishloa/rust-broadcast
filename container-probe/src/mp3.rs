//! MPEG audio Layer III (MP3) prober — ISO/IEC 11172-3 §2.4.2.3 (frame header).
//!
//! A frame begins with an 11-bit syncword (`b[0] == 0xFF`, `(b[1] & 0xE0) ==
//! 0xE0`) whose fields encode its own `frame_length`. Detection is by **length
//! chaining**: `144 * bitrate / sample_rate + padding` must land exactly on the
//! next frame's syncword. A genuine MP3 elementary stream chains 40+ frames; a
//! container merely contains scattered `0xFF` noise (measured: every corpus
//! container's longest MP3 chain is 0-1, `audio.mp3`'s is 44).
//!
//! Only MPEG-1 Layer III is matched (`MpegAudioFrameHeader::version()` is
//! `Mpeg1` and `layer()` is `LayerIII`); the reserved bitrate indices (0, 15)
//! and reserved sample-rate index (3) are rejected by the shared header parser.
//! An ID3v2 tag at the head is skipped before anchoring.
//!
//! Only the chain walk, the scoring and the ID3v2 skip live here: each
//! candidate header is judged by
//! `broadcast_bitstream::mpeg_audio::MpegAudioFrameHeader::parse`, the
//! workspace's one MPEG audio header parser.

use crate::{Confidence, Detail, Evidence, Outcome};
use broadcast_bitstream::{MpegAudioFrameHeader, MpegAudioLayer, MpegAudioVersion};

/// The ID3v2 tag signature (ID3v2 §3.1; the tag precedes the audio in
/// `fixtures/audio/audio.mp3`: `49 44 33 04`, version 2.4).
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
/// The bytes a 4-byte MPEG audio frame header occupies; a region shorter than
/// this cannot offer one header, so the prober cannot rule MP3 out.
const MP3_HEADER_LEN: usize = 4;
/// Minimum chained frames for a positive `LATTICE_WEAK` match.
const MP3_MIN_CHAIN_WEAK: usize = 4;
/// Chained frames that lift the verdict to `LATTICE_STRONG`.
const MP3_MIN_CHAIN_STRONG: usize = 16;

/// Byte offset of the ID3v2 header's flags field.
const ID3_FLAGS_OFFSET: usize = 5;
/// Byte offset of the ID3v2 syncsafe size field.
const ID3_SIZE_OFFSET: usize = 6;
/// Width of the syncsafe size field, in bytes.
const ID3_SIZE_LEN: usize = 4;
/// Payload bits each of those bytes carries (the top bit is always clear).
const ID3_SYNC_SAFE_BITS: u32 = 7;

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
        // No valid MP3 frame header was examined. That only rules MP3 out when
        // the region *was* long enough to have carried one: a leading ID3v2 tag
        // whose declared length reaches or extends past the region means the
        // audio frames begin at or beyond what the caller supplied, so the
        // prober ran out of data before it ever reached a frame position —
        // `Insufficient`, not `None`
        // (the shared `Insufficient` vs `Unknown` decision).
        if let Some(skip) = id3_skip(region)
            && skip >= region.len()
        {
            // The ID3 skip target is itself the structural need: the first
            // frame cannot start before it. The dropped `region.len() + 1`
            // term only mattered when the skip was already behind us, which
            // `normalise_need` now handles centrally.
            return Outcome::Insufficient(skip);
        }
        return Outcome::None;
    }
    // A partial chain of 1..=(WEAK-1) frames: `Insufficient` only when it ended
    // by running off the end of the buffer (`truncated`); a chain that ended at
    // an invalid next header is a scattered frame and will not become an MP3
    // stream by reading further. Mirrors the TS prober's `could_prove`.
    if truncated {
        Outcome::Insufficient(need_at_least(anchor, frame_len))
    } else {
        Outcome::None
    }
}

/// A lower bound on bytes that could resolve the verdict to `LATTICE_WEAK`:
/// room at the observed frame size for [`MP3_MIN_CHAIN_WEAK`] frames from the
/// chain's anchor, and strictly more than the caller already holds.
fn need_at_least(anchor: usize, frame_len: usize) -> usize {
    // Structural only -- see the note in `adts::need_at_least`.
    anchor.saturating_add(MP3_MIN_CHAIN_WEAK.saturating_mul(frame_len))
}

/// The longest chain of valid MPEG-1 Layer III frames linked by their own
/// `frame_length` fields, scanning from every candidate position (after
/// skipping any leading ID3v2 tag), together with whether that chain ended by
/// running off the buffer (`truncated`) or hitting an invalid next header, plus
/// its anchor offset and first-frame length. Short-circuits at
/// [`MP3_MIN_CHAIN_STRONG`], bounding the work.
fn longest_mp3_chain(data: &[u8]) -> crate::chain::Chain {
    crate::chain::longest_chain(
        data,
        id3_skip(data).unwrap_or(0),
        MP3_HEADER_LEN,
        MP3_MIN_CHAIN_STRONG,
        mp3_frame_len,
    )
}

/// `frame_length` for a valid MPEG-1 Layer III frame header at `i`, or `None`
/// if the bytes are not a conformant MP3 frame header.
///
/// The header itself is decoded by
/// `broadcast_bitstream::mpeg_audio::MpegAudioFrameHeader::parse` (the
/// workspace's one MPEG audio header parser). That parser also accepts MPEG-2,
/// MPEG-2.5 and Layers I/II, so the prober's own narrowing — MPEG-1 Layer III
/// only — is applied on top.
fn mp3_frame_len(data: &[u8], i: usize) -> Option<usize> {
    let header = MpegAudioFrameHeader::parse(data.get(i..)?).ok()?;
    if header.version() != MpegAudioVersion::Mpeg1 || header.layer() != MpegAudioLayer::LayerIII {
        return None;
    }
    Some(header.frame_length())
}

/// The number of bytes to skip past a leading ID3v2 tag, or `None` when there
/// is none (ID3v2 §3.1). A footer, when flagged, adds 10 bytes.
fn id3_skip(data: &[u8]) -> Option<usize> {
    if data.len() < ID3_HEADER_LEN || data[..ID3_SIGNATURE.len()] != ID3_SIGNATURE {
        return None;
    }
    let flags = data[ID3_FLAGS_OFFSET];
    let size = data[ID3_SIZE_OFFSET..ID3_SIZE_OFFSET + ID3_SIZE_LEN]
        .iter()
        .fold(0u32, |acc, &byte| {
            (acc << ID3_SYNC_SAFE_BITS) | u32::from(byte & ID3_SYNCSAFE_MASK)
        });
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
        test_fixtures::read(rel.trim_start_matches("fixtures/"))
    }

    /// The chain-threshold discriminator (mutation #2 for the MP3 chain rule).
    ///
    /// A genuine container file carries a single valid MPEG-1 Layer III frame
    /// somewhere in its payload (longest MP3 chain of 1), so with the real
    /// `MP3_MIN_CHAIN_WEAK = 4` the MP3 prober returns `None`. Dropping the
    /// threshold to 1 (raw frame counting) misidentifies the same file as Mp3.
    /// Observed under the mutation (`MP3_MIN_CHAIN_WEAK = 1`):
    /// ```
    /// h264-aac.ts must NOT match MP3 at the real threshold,
    ///   got Match(Evidence { confidence: Confidence(96), detail: None })
    /// ```
    #[test]
    fn chain_threshold_keeps_a_container_out() {
        let data = fixture_bytes("fixtures/ts/h264-aac.ts");
        match probe(&data, data.len()) {
            Outcome::None => {}
            other => panic!("h264-aac.ts must NOT match MP3 at the real threshold, got {other:?}"),
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
        let data = fixture_bytes("fixtures/audio/audio.mp3");
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
        let data = fixture_bytes("fixtures/audio/audio.mp3");
        let region = &data[..MP3_HEADER_LEN - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, MP3_HEADER_LEN),
            other => panic!("3-byte MP3 prefix must be Insufficient(4), got {other:?}"),
        }
    }

    /// The ID3-skip `Insufficient` path (round 4): a leading ID3v2 tag whose
    /// declared length extends past the supplied region means the audio frames
    /// begin beyond what the caller read, so the prober must ask for more bytes
    /// rather than ruling MP3 out (`None`, "stop") — there was no frame position
    /// to examine yet.
    ///
    /// The buffer below is an exact 10-byte ID3v2 header (version 2.4, no
    /// footer) whose syncsafe size declares a 127-byte tag + 10-byte header =
    /// 137 bytes total — far more than the 10 bytes supplied. `id3_skip` returns
    /// 137, `longest_mp3_chain` finds no frame (its scan starts past the region
    /// end), and the prober must answer `Insufficient(137)`.
    ///
    /// Observed under the mutation (the `if let Some(skip) … Insufficient` block
    /// deleted):
    /// ```
    /// ID3 tag longer than the region must be Insufficient, got None
    /// ```
    #[test]
    fn id3_tag_longer_than_the_region_is_insufficient() {
        // ID3v2.4 header, 127-byte syncsafe tag size: skip = 10 + 127 = 137.
        let data = [
            b'I', b'D', b'3', 0x04, 0x00, 0x00, // "ID3", v2.4, flags=0
            0x00, 0x00, 0x00, 0x7F, // syncsafe size = 127
        ];
        match probe(&data, data.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, 137),
            other => panic!("ID3 tag longer than the region must be Insufficient, got {other:?}"),
        }
    }

    /// An ID3v2 header with the given flags and 4-byte syncsafe size.
    fn id3_header(flags: u8, size: [u8; 4]) -> [u8; ID3_HEADER_LEN] {
        [
            b'I', b'D', b'3', 0x04, 0x00, flags, size[0], size[1], size[2], size[3],
        ]
    }

    /// The syncsafe size is seven bits from each of the four size bytes, most
    /// significant first: `[01 02 03 04]` is `1<<21 | 2<<14 | 3<<7 | 4` =
    /// 2 130 308 (ID3v2 §3.1), and the footer flag adds a 10-byte footer.
    #[test]
    fn id3_syncsafe_size_spans_all_four_bytes() {
        assert_eq!(
            id3_skip(&id3_header(0x00, [0x01, 0x02, 0x03, 0x04])),
            Some(2_130_318)
        );
        assert_eq!(
            id3_skip(&id3_header(ID3_FLAG_FOOTER, [0x01, 0x02, 0x03, 0x04])),
            Some(2_130_328)
        );
        // A size carried entirely by the high byte: 1 << 21 = 2 097 152.
        assert_eq!(
            id3_skip(&id3_header(0x00, [0x01, 0x00, 0x00, 0x00])),
            Some(2_097_162)
        );
    }

    /// A tag whose declared length exceeds the supplied region is a structural
    /// need, even when the size lives in the top syncsafe byte.
    #[test]
    fn a_tag_larger_than_the_region_is_a_structural_need() {
        let data = id3_header(0x00, [0x01, 0x00, 0x00, 0x00]);
        match probe(&data, data.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, 2_097_162),
            other => panic!("a 2 MiB ID3 tag must be Insufficient, got {other:?}"),
        }
    }

    /// A two-hundred-and-fifty-seven-byte tag body is skipped exactly, so the
    /// frame chain is found at the byte the tag declared (ID3v2 §3.1).
    #[test]
    fn a_multi_byte_tag_size_places_the_first_frame_after_the_tag() {
        let mut data = std::vec::Vec::from(id3_header(0x00, [0x00, 0x00, 0x02, 0x01]));
        data.resize(267, 0x00);
        let mut frame = std::vec![0xFF, 0xFB, 0x90, 0x00];
        frame.resize(417, 0x00);
        for _ in 0..MP3_MIN_CHAIN_WEAK {
            data.extend_from_slice(&frame);
        }
        assert_eq!(id3_skip(&data), Some(267));
        assert!(mp3_frame_len(&data, 267).is_some());
        assert_eq!(longest_mp3_chain(&data).2, 267);
        match probe(&data, data.len()) {
            Outcome::Match(_) => {}
            other => panic!("the frame after the tag must be seen, got {other:?}"),
        }
    }

    /// The footer flag moves the first frame another 10 bytes on.
    #[test]
    fn the_id3_footer_is_skipped() {
        let mut data = std::vec::Vec::from(id3_header(ID3_FLAG_FOOTER, [0x00, 0x00, 0x02, 0x01]));
        data.resize(277, 0x00);
        let mut frame = std::vec![0xFF, 0xFB, 0x90, 0x00];
        frame.resize(417, 0x00);
        data.extend_from_slice(&frame);
        assert_eq!(id3_skip(&data), Some(277));
        assert!(mp3_frame_len(&data, 277).is_some());
        assert_eq!(longest_mp3_chain(&data).2, 277);
    }

    /// `frame_length = 144 * bitrate / sample_rate + padding` (ISO/IEC
    /// 11172-3 §2.4.2.3): 144 * 128 000 / 44 100 is 417, plus the padding bit —
    /// which is bit 1 of the header's THIRD byte, not of the fourth. The fourth
    /// byte's bit 1 is the low bit of `emphasis` and must not lengthen a frame.
    #[test]
    fn a_frame_length_matches_the_iso_table_with_and_without_padding() {
        assert_eq!(mp3_frame_len(&[0xFF, 0xFB, 0x90, 0x00], 0), Some(417));
        assert_eq!(mp3_frame_len(&[0xFF, 0xFB, 0x92, 0x00], 0), Some(418));
        assert_eq!(
            mp3_frame_len(&[0xFF, 0xFB, 0x90, 0x02], 0),
            Some(417),
            "emphasis bit 1 is not padding"
        );
    }

    /// Each reserved or out-of-range header field is rejected on its own.
    #[test]
    fn invalid_frame_header_fields_are_rejected() {
        for header in [
            [0xFF, 0x1A, 0x90, 0x00], // syncword bits 7:5 clear
            [0xFF, 0xF2, 0x90, 0x00], // version 10 (MPEG-2)
            [0xFF, 0xEB, 0x90, 0x00], // version 01 (reserved)
            [0xFF, 0xF9, 0x90, 0x00], // layer 00 (reserved)
            [0xFF, 0xFB, 0xF0, 0x00], // bitrate index 1111 (forbidden)
            [0xFF, 0xFB, 0x00, 0x00], // bitrate index 0000 (free format)
            [0xFF, 0xFB, 0x9C, 0x00], // sample-rate index 11 (reserved)
        ] {
            assert_eq!(mp3_frame_len(&header, 0), None, "{header:?}");
        }
    }

    /// One truncated frame is a structural need for four frames at its own
    /// length, not a fixed or length-relative figure.
    #[test]
    fn a_single_truncated_frame_asks_for_a_weak_chain() {
        let data = [0xFF, 0xFB, 0x90, 0x00];
        match probe(&data, data.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, 1_668),
            other => panic!("one 417-byte frame must ask for four (1668), got {other:?}"),
        }
    }

    /// A region that holds exactly the ID3 tag ends at the first audio byte;
    /// more bytes could still reveal the frame, so this is a short read, not a
    /// definitive rejection.
    #[test]
    fn a_region_that_is_exactly_the_id3_tag_asks_for_the_frames() {
        let mut data = std::vec::Vec::from(id3_header(0x00, [0x00, 0x00, 0x02, 0x01]));
        data.resize(267, 0x00);
        match probe(&data, data.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, 267),
            other => panic!(
                "a region ending exactly at the tag's audio start is a short read, got {other:?}"
            ),
        }
    }
}
