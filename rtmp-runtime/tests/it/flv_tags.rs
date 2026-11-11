//! Byte-exact FLV header/tag codec round trips over the committed FLV fixtures
//! (Adobe FLV v10.1 Annex E §E.2, §E.4.1).

use broadcast_common::{Parse, Serialize};
use rtmp_runtime::flv::{
    FLV_HEADER_LEN, FLV_PREV_TAG_SIZE_LEN, FLV_TAG_HEADER_LEN, FlvHeader, FlvTag, TagType,
};

fn fixtures() -> Vec<(&'static str, Vec<u8>)> {
    ["flv/av.flv", "flv/aac-7-1.flv", "flv/oversize-dims.flv"]
        .into_iter()
        .map(|name| (name, test_fixtures::read(name)))
        .collect()
}

#[test]
fn every_fixture_round_trips_byte_exactly() {
    for (name, bytes) in fixtures() {
        let header = FlvHeader::parse(&bytes).unwrap_or_else(|e| panic!("{name} header: {e:?}"));
        let mut out = header.try_to_bytes().unwrap();
        let mut count = 0usize;
        for tag in rtmp_runtime::flv::tags(&bytes, &header) {
            let tag = tag.unwrap_or_else(|e| panic!("{name} tag {count}: {e:?}"));
            out.extend_from_slice(&tag.try_to_bytes().unwrap());
            count += 1;
        }
        assert!(count > 0, "{name} must carry tags");
        assert_eq!(out, bytes, "{name} must serialize back byte-for-byte");
    }
}

#[test]
fn parsed_tags_reach_the_end_of_the_file() {
    for (name, bytes) in fixtures() {
        let header = FlvHeader::parse(&bytes).unwrap();
        let mut end = header.first_tag_offset();
        for tag in rtmp_runtime::flv::tags(&bytes, &header) {
            end = end.saturating_add(tag.unwrap().serialized_len());
        }
        assert_eq!(end, bytes.len(), "{name} tags must cover the file");
    }
}

#[test]
fn a_reserved_tag_type_keeps_its_byte() {
    let tag = FlvTag {
        tag_type: TagType::Reserved(0x63),
        timestamp: 0,
        stream_id: 0,
        body: &[],
    };
    let bytes = tag.try_to_bytes().unwrap();
    assert_eq!(bytes.first().copied(), Some(0x63));
    assert_eq!(
        FlvTag::parse(&bytes).unwrap().tag_type,
        TagType::Reserved(0x63)
    );
}

#[test]
fn mutating_a_tag_timestamp_extended_byte_changes_the_output() {
    let bytes = test_fixtures::read("flv/av.flv");
    let header = FlvHeader::parse(&bytes).unwrap();
    let tail = &bytes[header.first_tag_offset()..];
    let mut tag = FlvTag::parse(tail).unwrap();
    let original = tag.try_to_bytes().unwrap();

    tag.timestamp ^= 1 << 24;
    let mutated = tag.try_to_bytes().unwrap();
    assert_ne!(mutated, original, "timestamp extended byte must be written");
    let extended_at = FLV_TAG_HEADER_LEN - 4;
    assert_ne!(mutated[extended_at], original[extended_at]);
    assert_eq!(
        mutated[..extended_at],
        original[..extended_at],
        "only the extended timestamp byte moves"
    );

    let reparsed = FlvTag::parse(&mutated).unwrap();
    assert_eq!(reparsed.timestamp, tag.timestamp);
}

#[test]
fn truncated_input_at_every_length_is_an_error_not_a_panic() {
    for (_name, bytes) in fixtures() {
        for len in 0..bytes.len() {
            let prefix = &bytes[..len];
            let Ok(header) = FlvHeader::parse(prefix) else {
                continue;
            };
            for tag in rtmp_runtime::flv::tags(prefix, &header) {
                if tag.is_err() {
                    break;
                }
            }
        }
    }
}

#[test]
fn a_short_trailing_tag_is_reported() {
    let bytes = test_fixtures::read("flv/av.flv");
    let header = FlvHeader::parse(&bytes).unwrap();
    let cut = &bytes[..bytes.len() - 1];
    let mut saw_error = false;
    for tag in rtmp_runtime::flv::tags(cut, &header) {
        if tag.is_err() {
            saw_error = true;
        }
    }
    assert!(saw_error, "a cut final tag must be an error item");
}

#[test]
fn header_round_trips_a_non_standard_data_offset() {
    let mut bytes = FlvHeader::new(true, false).try_to_bytes().unwrap();
    let offset = u32::try_from(FLV_HEADER_LEN).unwrap() + 4;
    bytes[5..FLV_HEADER_LEN].copy_from_slice(&offset.to_be_bytes());
    let header = FlvHeader::parse(&bytes).unwrap();
    assert_eq!(header.data_offset, offset);
    assert!(!header.has_video);
    assert_eq!(header.try_to_bytes().unwrap(), bytes);

    let first_tag_at = FLV_HEADER_LEN + FLV_PREV_TAG_SIZE_LEN;
    let tag = FlvTag {
        tag_type: TagType::Audio,
        timestamp: 0,
        stream_id: 0,
        body: &[0xAF],
    };
    let mut stream = bytes.clone();
    stream.resize(usize::try_from(offset).unwrap(), 0);
    stream.extend_from_slice(&[0; FLV_PREV_TAG_SIZE_LEN]);
    stream.extend_from_slice(&tag.try_to_bytes().unwrap());
    assert!(stream.len() > first_tag_at);
    let walked: Vec<_> = rtmp_runtime::flv::tags(&stream, &header).collect();
    assert_eq!(walked.len(), 1);
    assert_eq!(walked[0].as_ref().map(|t| t.body), Ok(&[0xAF][..]));
}

#[test]
fn a_bad_signature_or_short_data_offset_is_malformed() {
    let mut bytes = FlvHeader::new(true, true).try_to_bytes().unwrap();
    bytes[0] = b'X';
    assert!(FlvHeader::parse(&bytes).is_err());

    let mut bytes = FlvHeader::new(true, true).try_to_bytes().unwrap();
    bytes[5..FLV_HEADER_LEN].copy_from_slice(&8u32.to_be_bytes());
    assert!(FlvHeader::parse(&bytes).is_err());
}
