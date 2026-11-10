//! ISO/IEC 14496-12:2026 §8.16 compressed boxes (`otyp`, `!mov`, `!mof`): real
//! fixtures with their top-level `moov`/`moof` replaced by the compressed form.
//! The prober never inflates, so the payload is a valid RFC 1951 stored-block
//! stream of the original payload.

use container_probe::segment::{media_fragment, progressive_movie};
use container_probe::{Confidence, Detail::Isobmff, Format, IsobmffLayout, Probe};

const COMPRESSED_MOVIE: &[u8; 4] = b"!mov";
const COMPRESSED_FRAGMENT: &[u8; 4] = b"!mof";
const COMPRESSED_SEGMENT_INDEX: &[u8; 4] = b"!six";
const COMPRESSED_SUBSEGMENT_INDEX: &[u8; 4] = b"!ssx";
const ORIGINAL_FILE_TYPE: &[u8; 4] = b"otyp";
const BOX_HEADER: usize = 8;
const STORED_BLOCK_MAX: usize = 0xFFFF;
const FINAL_STORED_BLOCK: u8 = 0x01;
const NONFINAL_STORED_BLOCK: u8 = 0x00;

fn deflate_stored(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunks = payload.chunks(STORED_BLOCK_MAX).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[FINAL_STORED_BLOCK, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(chunk) = chunks.next() {
        out.push(if chunks.peek().is_none() {
            FINAL_STORED_BLOCK
        } else {
            NONFINAL_STORED_BLOCK
        });
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((BOX_HEADER + body.len()) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(kind);
    b.extend_from_slice(body);
    b
}

/// Replace each top-level `moov` / `moof` with its compressed form and, when a
/// compressed box was written, put an `otyp` holding the original `ftyp`/`styp`
/// right after it (§8.16.5).
fn compress_top_level(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + BOX_HEADER <= data.len() {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        assert!(size >= BOX_HEADER, "fixture uses only compact boxes");
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap();
        let whole = &data[at..at + size];
        match &kind {
            b"moov" => out.extend(boxed(
                COMPRESSED_MOVIE,
                &deflate_stored(&whole[BOX_HEADER..]),
            )),
            b"moof" => out.extend(boxed(
                COMPRESSED_FRAGMENT,
                &deflate_stored(&whole[BOX_HEADER..]),
            )),
            b"ftyp" | b"styp" => {
                out.extend_from_slice(whole);
                out.extend(boxed(ORIGINAL_FILE_TYPE, whole));
            }
            _ => out.extend_from_slice(whole),
        }
        at += size;
    }
    out
}

fn layout(data: &[u8]) -> (Confidence, IsobmffLayout) {
    match container_probe::probe(data) {
        Probe::Identified {
            format: Format::Isobmff,
            confidence,
            detail: Isobmff { layout, .. },
            ..
        } => (confidence, layout),
        other => panic!("expected Isobmff, got {other:?}"),
    }
}

#[test]
fn a_compressed_movie_is_a_progressive_movie() {
    let bytes = compress_top_level(&test_fixtures::read("mp4/progressive/av-faststart.mp4"));
    assert!(bytes.windows(4).any(|w| w == COMPRESSED_MOVIE));
    assert_eq!(
        layout(&bytes),
        (Confidence::STRUCTURAL, IsobmffLayout::Progressive)
    );
    assert!(progressive_movie(&bytes));
    assert!(!media_fragment(&bytes));
}

#[test]
fn a_compressed_fragment_is_a_media_fragment() {
    let bytes = compress_top_level(&test_fixtures::read("hls/cmaf-fmp4/index0.m4s"));
    assert!(bytes.windows(4).any(|w| w == COMPRESSED_FRAGMENT));
    assert_eq!(layout(&bytes).1, IsobmffLayout::Fragmented);
    assert!(media_fragment(&bytes));
    assert!(!progressive_movie(&bytes));
}

#[test]
fn a_compressed_init_segment_is_a_movie_and_a_compressed_moof_ends_its_progressive_claim() {
    let init = compress_top_level(&test_fixtures::read("hls/cmaf-fmp4/init.mp4"));
    assert!(progressive_movie(&init));
    let mut both = boxed(COMPRESSED_MOVIE, &deflate_stored(b"m"));
    both.extend(boxed(COMPRESSED_FRAGMENT, &deflate_stored(b"f")));
    assert!(!progressive_movie(&both));
}

#[test]
fn a_file_that_starts_with_a_compressed_box_is_isobmff() {
    for (kind, expected) in [
        (COMPRESSED_MOVIE, IsobmffLayout::Progressive),
        (COMPRESSED_FRAGMENT, IsobmffLayout::Fragmented),
    ] {
        let bytes = boxed(kind, &deflate_stored(&[0u8; 64]));
        match container_probe::probe(&bytes) {
            Probe::Identified {
                format: Format::Isobmff,
                confidence,
                detail: Isobmff { layout, .. },
                ..
            } => {
                assert_eq!(confidence, Confidence::HEURISTIC);
                assert_eq!(layout, expected);
            }
            other => panic!("{kind:?}: expected Isobmff, got {other:?}"),
        }
    }
}

#[test]
fn a_file_that_starts_with_a_compressed_index_box_is_isobmff() {
    for kind in [COMPRESSED_SEGMENT_INDEX, COMPRESSED_SUBSEGMENT_INDEX] {
        let bytes = boxed(kind, &deflate_stored(&[0u8; 64]));
        match container_probe::probe(&bytes) {
            container_probe::Probe::Identified {
                format: container_probe::Format::Isobmff,
                confidence: container_probe::Confidence::HEURISTIC,
                ..
            } => {}
            other => panic!("{kind:?}: expected Isobmff, got {other:?}"),
        }
    }
}
