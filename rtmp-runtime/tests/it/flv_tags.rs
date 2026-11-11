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
fn a_cut_trailing_tag_body_is_reported() {
    let bytes = test_fixtures::read("flv/av.flv");
    let header = FlvHeader::parse(&bytes).unwrap();
    let cut = &bytes[..bytes.len() - 40];
    let mut saw_error = false;
    for tag in rtmp_runtime::flv::tags(cut, &header) {
        if tag.is_err() {
            saw_error = true;
        }
    }
    assert!(saw_error, "a cut final tag body must be an error item");

    // §E.4.1 makes the trailing PreviousTagSize redundant (it repeats the
    // tag's own size), so a final tag missing only that field is complete.
    let complete = &bytes[..bytes.len() - FLV_PREV_TAG_SIZE_LEN];
    let tags: Vec<_> = rtmp_runtime::flv::tags(complete, &header)
        .collect::<Result<Vec<_>, _>>()
        .expect("a final tag missing only its redundant PreviousTagSize is complete");
    let full: Vec<_> = rtmp_runtime::flv::tags(&bytes, &header)
        .collect::<Result<Vec<_>, _>>()
        .expect("the whole fixture walks");
    assert_eq!(
        tags.len(),
        full.len(),
        "every tag is walked; only the final trailer is missing"
    );
    assert!(!tags.is_empty(), "the fixture carries tags");
    let last = tags.last().expect("the fixture carries tags");
    let last_full = full.last().expect("the fixture carries tags");
    assert_eq!(last.tag_type, last_full.tag_type, "final tag type");
    assert_eq!(last.timestamp, last_full.timestamp, "final tag timestamp");
    assert_eq!(last.body, last_full.body, "final tag body");
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

// ---------------------------------------------------------------------------
// `tags_resync`: the tolerant walk (transmux's `FlvDemux` adapts it)
// ---------------------------------------------------------------------------

/// A body long enough that a tag's header, body and `PreviousTagSize` are all
/// distinct from a neighbouring tag's framing.
const RESYNC_BODY_LEN: usize = 32;
/// How much of a two-tag stream is cut away: part of the final tag's body and
/// all of its `PreviousTagSize`.
const RESYNC_TAIL_CUT: usize = 16;

/// The byte offset of every tag in `stream`, in order.
fn tag_starts(stream: &[u8], header: &FlvHeader) -> Vec<usize> {
    let mut off = header.first_tag_offset();
    let mut starts = Vec::new();
    for tag in rtmp_runtime::flv::tags(stream, header) {
        starts.push(off);
        off += tag.unwrap().serialized_len();
    }
    starts
}

#[test]
fn tags_resync_skips_a_damaged_tag_and_reports_its_offset() {
    let name = "flv/av.flv";
    let bytes = test_fixtures::read(name);
    let header = FlvHeader::parse(&bytes).unwrap_or_else(|e| panic!("{name} header: {e:?}"));
    let starts = tag_starts(&bytes, &header);
    let mut stream = bytes.clone();
    let victim = starts[1];
    let damage_at = victim + FLV_TAG_HEADER_LEN;
    stream[damage_at..damage_at + 4].fill(0xAA);
    let inflated = u32::try_from(stream.len() - victim).unwrap();
    stream[victim + 1..victim + 4].copy_from_slice(&inflated.to_be_bytes()[1..]);

    let walk = rtmp_runtime::flv::tags_resync(&stream, &header)
        .unwrap_or_else(|e| panic!("a damaged tag is recoverable: {e:?}"));
    assert_eq!(
        walk.damaged,
        vec![victim],
        "the damaged tag is skipped and reported at its offset"
    );
    let offsets: Vec<usize> = walk.tags.iter().map(|(off, _)| *off).collect();
    let expected: Vec<usize> = starts
        .iter()
        .copied()
        .filter(|&off| off != victim)
        .collect();
    assert_eq!(
        offsets, expected,
        "every tag outside the damaged one survives, at its own offset"
    );
    assert_eq!(walk.truncated_tail, None);
}

#[test]
fn tags_resync_flags_a_truncated_tail() {
    let mut stream = FlvHeader::new(false, true).try_to_bytes().unwrap();
    let good = FlvTag::new(TagType::from_byte(9), 0, &[1u8, 2, 3, 4]);
    stream.extend_from_slice(&good.try_to_bytes().unwrap());
    let first_at = tag_starts(&stream, &FlvHeader::parse(&stream).unwrap())[0];
    let cut_off = FlvTag::new(TagType::from_byte(9), 0, &[0xAAu8; RESYNC_BODY_LEN]);
    stream.extend_from_slice(&cut_off.try_to_bytes().unwrap());
    stream.truncate(stream.len() - RESYNC_TAIL_CUT);

    let header = FlvHeader::parse(&stream).unwrap();
    let walk = rtmp_runtime::flv::tags_resync(&stream, &header).expect("the good prefix survives");
    assert_eq!(walk.tags.len(), 1, "the complete first tag is kept");
    let (at, need, have) = walk
        .truncated_tail
        .expect("the final, cut-off tag is flagged");
    assert_eq!(at, first_at + good.serialized_len());
    assert!(
        need > have,
        "need {need} must exceed the {have} bytes present"
    );
}

#[test]
fn tags_resync_over_a_header_only_input_yields_no_tags() {
    let stream = FlvHeader::new(false, true).try_to_bytes().unwrap();
    let header = FlvHeader::parse(&stream).unwrap();
    let walk =
        rtmp_runtime::flv::tags_resync(&stream, &header).expect("header and no tags is no error");
    assert!(walk.tags.is_empty());
    assert!(walk.damaged.is_empty());
    assert_eq!(walk.truncated_tail, None);
}
