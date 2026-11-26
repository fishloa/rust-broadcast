//! Real-file round trips: every committed fixture walks to the end and
//! re-serialises byte for byte, element by element.

use broadcast_core::{Parse, Serialize};
use broadcast_matroska::Limits;
use broadcast_matroska::ebml::document::parse_document_start;
use broadcast_matroska::matroska::{Block, Document, Element, ids};

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
            for element in cluster.children() {
                let payload = match element.id() {
                    ids::SIMPLE_BLOCK => element.as_binary().map(|binary| binary.bytes()),
                    ids::BLOCK_GROUP => element
                        .find(ids::BLOCK)
                        .and_then(|block| block.as_binary())
                        .map(|binary| binary.bytes()),
                    _ => None,
                };
                let Some(payload) = payload else {
                    continue;
                };
                let block = Block::parse(payload)
                    .unwrap_or_else(|error| panic!("{fixture}: block: {error}"));
                assert_eq!(
                    block.try_to_bytes().unwrap(),
                    payload,
                    "{fixture}: block {:X} bytes",
                    element.id()
                );
                blocks += 1;
            }
        }
        assert!(blocks > 0, "{fixture}: no blocks found");
    }
}

#[test]
fn unknown_sized_segment_and_cluster_round_trip() {
    use broadcast_matroska::ebml::ElementId;
    use broadcast_matroska::ebml::document::EBML_ID;
    use broadcast_matroska::ebml::vint::VintSize;

    fn framed(id: u32, payload: &[u8]) -> Vec<u8> {
        let id = ElementId::try_from_raw(id).unwrap();
        let mut out = vec![0u8; 12];
        let id_len = id.write(&mut out).unwrap();
        let size = VintSize::minimal(u64::try_from(payload.len()).unwrap()).unwrap();
        let size_len = size.write(&mut out[id_len..]).unwrap();
        out.truncate(id_len + size_len);
        out.extend_from_slice(payload);
        out
    }
    fn unknown_framed(id: u32) -> Vec<u8> {
        let id = ElementId::try_from_raw(id).unwrap();
        let mut out = vec![0u8; 5];
        let id_len = id.write(&mut out).unwrap();
        out[id_len] = 0xFF;
        out.truncate(id_len + 1);
        out
    }

    let mut file = framed(EBML_ID, &[]);
    file.extend_from_slice(&unknown_framed(ids::SEGMENT));
    let unknown_child = framed(0x4FFF, b"opaque");
    file.extend_from_slice(&framed(ids::INFO, &unknown_child));
    file.extend_from_slice(&unknown_framed(ids::CLUSTER));
    file.extend_from_slice(&framed(ids::TIMESTAMP, &[0x00]));
    file.extend_from_slice(&framed(ids::SIMPLE_BLOCK, &[0x81, 0x00, 0x00, 0x80, 0xAA]));
    file.extend_from_slice(&framed(ids::CUES, &[]));

    let document = Document::parse(&file).unwrap();
    assert_eq!(document.try_to_bytes().unwrap(), file);
    let segment = document.segment();
    assert_eq!(
        segment.header().payload_len(),
        None,
        "Segment stays unknown-size"
    );
    let cluster = segment.clusters().next().expect("one cluster");
    assert_eq!(
        cluster.header().payload_len(),
        None,
        "Cluster stays unknown-size"
    );
    assert_eq!(cluster.timestamp(), Some(0));
    let info = segment.info().expect("info");
    let unknown = info.children().first().expect("unknown child");
    assert_eq!(unknown.id(), 0x4FFF);
    match unknown {
        Element::Unknown(value) => assert_eq!(value.payload(), b"opaque"),
        other => panic!("expected an untyped element, got {:X}", other.id()),
    }
    assert!(
        segment.cues().is_some(),
        "Cues ended the unknown-size Cluster"
    );
}
