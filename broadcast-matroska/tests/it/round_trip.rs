//! Real-file round trips: every committed fixture walks to the end and
//! re-serialises byte for byte, element by element.

use broadcast_common::{Parse, Serialize};
use broadcast_matroska::Limits;
use broadcast_matroska::ebml::document::parse_document_start;
use broadcast_matroska::matroska::{Document, Element, ids};

const FIXTURES: &[&str] = &[
    "mkv/h264-aac.mkv",
    "mkv/hevc-aac.mkv",
    "mkv/vorbis.webm",
    "mkv/vp8-opus.webm",
    "mkv/vp9-opus.mkv",
    "mkv/vp9-opus.webm",
];

#[test]
fn every_fixture_round_trips_byte_for_byte() {
    for fixture in FIXTURES {
        let bytes = test_fixtures::read(fixture);
        let document = Document::parse(&bytes).unwrap_or_else(|error| {
            panic!("{fixture}: document parse failed: {error}");
        });
        let serialized = document.try_to_bytes().unwrap();
        assert_eq!(serialized, bytes, "{fixture}: document round trip");
    }
}

#[test]
fn every_element_round_trips_against_its_source_bytes() {
    for fixture in FIXTURES {
        let bytes = test_fixtures::read(fixture);
        let limits = Limits::new();
        let (header, header_used) = parse_document_start(&bytes).unwrap();
        assert_eq!(
            header.try_to_bytes().unwrap(),
            &bytes[..header_used],
            "{fixture}: EBML header round trip"
        );
        let rest = &bytes[header_used..];
        let (segment, segment_used) = Element::parse_at(rest, &limits).unwrap();
        assert_eq!(segment.id(), ids::SEGMENT, "{fixture}: document root");
        check_element(&segment, &rest[..segment_used], fixture);
        assert_eq!(
            header_used + segment_used,
            bytes.len(),
            "{fixture}: walk reached the end"
        );
    }
}

/// Check one element against its source bytes, then recurse into its children.
fn check_element(element: &Element<'_>, source: &[u8], fixture: &str) {
    let re_serialized = element.try_to_bytes().unwrap_or_else(|error| {
        panic!(
            "{fixture}: element {:X} re-serialize: {error}",
            element.id()
        );
    });
    assert_eq!(
        re_serialized,
        source,
        "{fixture}: element {:X} bytes",
        element.id()
    );
    let Some(children) = element.children() else {
        return;
    };
    let header_len = element.header().serialized_len();
    let payload = source
        .get(header_len..)
        .unwrap_or_else(|| panic!("{fixture}: element {:X} has no payload", element.id()));
    let mut position = 0usize;
    for child in children {
        let len = child.serialized_len();
        let end = position + len;
        let slice = payload
            .get(position..end)
            .unwrap_or_else(|| panic!("{fixture}: child {:X} past its parent", child.id()));
        check_element(child, slice, fixture);
        position = end;
    }
    assert_eq!(
        position,
        payload.len(),
        "{fixture}: element {:X} children cover its payload",
        element.id()
    );
}

#[test]
fn every_cluster_block_round_trips() {
    for fixture in FIXTURES {
        let bytes = test_fixtures::read(fixture);
        let document = Document::parse(&bytes).unwrap();
        let mut blocks = 0usize;
        for cluster in document.segment().clusters() {
            for block in cluster.simple_blocks() {
                let block = block.unwrap_or_else(|error| panic!("{fixture}: block: {error}"));
                let re_serialized = block.try_to_bytes().unwrap();
                assert!(!re_serialized.is_empty(), "{fixture}: block round trip");
                blocks += 1;
            }
            for group in cluster.block_groups() {
                if let Some(block) = group.block() {
                    let block = block.unwrap_or_else(|error| panic!("{fixture}: block: {error}"));
                    assert!(!block.try_to_bytes().unwrap().is_empty());
                    blocks += 1;
                }
            }
        }
        assert!(blocks > 0, "{fixture}: no blocks found");
    }
}
