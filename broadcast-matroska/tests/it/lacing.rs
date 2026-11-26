//! All three lacing forms built from a real frame payload, plus the fixture's
//! own blocks (RFC 9559 §12).

use broadcast_core::{Parse, Serialize};
use broadcast_matroska::ebml::vint::{VintSize, VintWidth};
use broadcast_matroska::matroska::block::{
    FLAG_KEYFRAME, FLAG_LACING_EBML, FLAG_LACING_FIXED, FLAG_LACING_XIPH, XIPH_CONTINUATION,
};
use broadcast_matroska::matroska::{Block, BlockHeader, Document, Lacing, LacingKind};

#[test]
fn real_fixture_blocks_round_trip_their_lacing() {
    let bytes = test_fixtures::read("mkv/vp9-opus.webm");
    let document = Document::parse(&bytes).unwrap();
    let mut seen = 0usize;
    for cluster in document.segment().clusters() {
        for element in cluster.children() {
            if element.id() != broadcast_matroska::matroska::ids::SIMPLE_BLOCK {
                continue;
            }
            let payload = element.as_binary().unwrap().bytes();
            let block = Block::parse(payload).unwrap();
            assert_eq!(block.try_to_bytes().unwrap(), payload);
            seen += 1;
        }
    }
    assert!(seen > 0, "fixture carries no SimpleBlock");
}

#[test]
fn xiph_lacing_round_trips_a_real_frame() {
    let frame = real_frame();
    let (a, rest) = frame.split_at(frame.len() / 3);
    let (b, c) = rest.split_at(rest.len() / 2);
    let mut payload = header_bytes(FLAG_KEYFRAME | FLAG_LACING_XIPH);
    payload.push(2);
    for part in [a, b] {
        let mut remaining = part.len();
        while remaining >= usize::from(XIPH_CONTINUATION) {
            payload.push(XIPH_CONTINUATION);
            remaining -= usize::from(XIPH_CONTINUATION);
        }
        payload.push(u8::try_from(remaining).unwrap());
    }
    payload.extend_from_slice(a);
    payload.extend_from_slice(b);
    payload.extend_from_slice(c);
    let block = Block::parse(&payload).unwrap();
    assert_eq!(block.lacing().kind(), LacingKind::Xiph);
    assert_eq!(block.frame_count(), 3);
    assert_eq!(block.frame(0), Some(a));
    assert_eq!(block.frame(1), Some(b));
    assert_eq!(block.frame(2), Some(c));
    assert_eq!(block.try_to_bytes().unwrap(), payload);
}

#[test]
fn ebml_lacing_round_trips_a_real_frame() {
    let frame = real_frame();
    let (a, rest) = frame.split_at(frame.len() / 3);
    let (b, c) = rest.split_at(rest.len() / 2);
    let mut payload = header_bytes(FLAG_KEYFRAME | FLAG_LACING_EBML);
    payload.push(2);
    let first = VintSize::Known {
        value: u64::try_from(a.len()).unwrap(),
        width: VintWidth::try_new(2).unwrap(),
    };
    let mut scratch = [0u8; 8];
    let first_len = first.write(&mut scratch).unwrap();
    payload.extend_from_slice(&scratch[..first_len]);
    let delta = i64::try_from(b.len()).unwrap() - i64::try_from(a.len()).unwrap();
    let biased = delta + 63;
    payload.push(0x80 | u8::try_from(biased).unwrap());
    payload.extend_from_slice(a);
    payload.extend_from_slice(b);
    payload.extend_from_slice(c);
    let block = Block::parse(&payload).unwrap();
    assert_eq!(block.lacing().kind(), LacingKind::Ebml);
    assert_eq!(block.frame_count(), 3);
    assert_eq!(block.frame(0), Some(a));
    assert_eq!(block.frame(1), Some(b));
    assert_eq!(block.frame(2), Some(c));
    assert_eq!(block.try_to_bytes().unwrap(), payload);
}

#[test]
fn fixed_lacing_round_trips_a_real_frame() {
    let mut payload = header_bytes(FLAG_KEYFRAME | FLAG_LACING_FIXED);
    let owned = real_frame();
    let frame = &owned[..9];
    let parts: Vec<&[u8]> = frame.chunks(3).collect();
    assert_eq!(parts.len(), 3);
    payload.push(2);
    payload.extend_from_slice(frame);
    let block = Block::parse(&payload).unwrap();
    assert_eq!(block.lacing().kind(), LacingKind::Fixed);
    assert_eq!(block.frame_count(), 3);
    for (index, part) in parts.iter().enumerate() {
        assert_eq!(block.frame(index), Some(*part));
    }
    assert_eq!(block.try_to_bytes().unwrap(), payload);
    match block.lacing() {
        Lacing::Fixed { frames } => assert_eq!(frames.len(), 3),
        other => panic!("expected fixed lacing, got {}", other.kind()),
    }
}

/// A real coded frame from the WebM fixture.
fn real_frame() -> Vec<u8> {
    let bytes = test_fixtures::read("mkv/vp9-opus.webm");
    let document = Document::parse(&bytes).unwrap();
    for cluster in document.segment().clusters() {
        for block in cluster.simple_blocks() {
            let block = block.unwrap();
            if block.frame_count() == 1 {
                return block.frame(0).unwrap().to_vec();
            }
        }
    }
    panic!("fixture carries no single-frame SimpleBlock");
}

/// A Block header for track 1 at timestamp 0 with `flags`.
fn header_bytes(flags: u8) -> Vec<u8> {
    let header = BlockHeader::with_values(1, 0, flags).unwrap();
    let mut out = vec![0u8; header.serialized_len()];
    header.write(&mut out).unwrap();
    out
}
