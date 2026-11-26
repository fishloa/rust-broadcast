//! Tier 1 boxes found in real files written by GPAC 26.07 (`MP4Box`), with the
//! values cross-checked against `mp4dump` for the same file.
//!
//! `fixtures/mp4/boxes/PROVENANCE.md` records the source, the exact command and
//! the tool version for each file. Every typed box is serialized back and
//! compared byte-for-byte with the bytes in the fixture.

use broadcast_core::{Parse, Serialize};
use isobmff::metadata::{Udta, UdtaChild};
use isobmff::{BoxIter, MovieBox, parse_box};

fn moov_of(file: &[u8]) -> Vec<u8> {
    for item in BoxIter::new(file) {
        let item = item.expect("top-level box");
        if item.four_cc() == *b"moov" {
            return item.bytes.to_vec();
        }
    }
    panic!("fixture has no moov");
}

fn children(parent: &[u8], fourcc: [u8; 4]) -> Vec<Vec<u8>> {
    let item = parse_box(parent).expect("box header").0;
    item.children()
        .filter_map(|child| child.ok())
        .filter(|child| child.four_cc() == fourcc)
        .map(|child| child.bytes.to_vec())
        .collect()
}

fn one_child(parent: &[u8], fourcc: [u8; 4]) -> Vec<u8> {
    let mut found = children(parent, fourcc);
    assert_eq!(found.len(), 1, "exactly one {fourcc:?}");
    found.pop().expect("one")
}

#[test]
fn track_description_fixture_carries_elng_kind_and_trgr() {
    let file = test_fixtures::read("mp4/boxes/track-description.mp4");
    let moov = moov_of(&file);
    let movie = MovieBox::parse(&moov).expect("moov parses");
    assert_eq!(movie.tracks.len(), 2);

    let traks = children(&moov, *b"trak");
    assert_eq!(traks.len(), 2);
    let audio_trak = &traks[1];

    // mp4dump: `[elng] size=8+10` on the audio track.
    let elng = movie.tracks[1]
        .mdia
        .as_ref()
        .and_then(|mdia| mdia.elng.as_ref())
        .expect("audio track elng");
    assert_eq!(elng.language_str(), Some("en-US"));
    assert_eq!(
        elng.try_to_bytes().expect("elng bytes"),
        one_child(&one_child(audio_trak, *b"mdia"), *b"elng")
    );

    // mp4dump: `[kind] size=8+33`; scheme URI and value as written.
    let udta_bytes = one_child(audio_trak, *b"udta");
    let udta = Udta::parse(&udta_bytes).expect("udta parses");
    let kind = udta
        .children
        .iter()
        .find_map(|child| match child {
            UdtaChild::Kind(kind) => Some(kind),
            _ => None,
        })
        .expect("kind");
    assert_eq!(kind.scheme_uri_str(), Some("urn:mpeg:dash:role:2011"));
    assert_eq!(kind.value_str(), Some("main"));
    assert_eq!(
        kind.try_to_bytes().expect("kind bytes"),
        one_child(&udta_bytes, *b"kind")
    );

    // mp4dump: `[trgr] size=8+16` on both tracks, a `msrc` group with id 1.
    for (index, track) in movie.tracks.iter().enumerate() {
        let trgr = track.trgr.as_ref().expect("trgr");
        assert_eq!(trgr.groups().len(), 1);
        assert_eq!(trgr.groups()[0].group_type(), *b"msrc");
        assert_eq!(trgr.groups()[0].group_id(), 1);
        assert_eq!(
            trgr.try_to_bytes().expect("trgr bytes"),
            one_child(&traks[index], *b"trgr")
        );
    }
}

#[test]
fn gpac_ster_group_without_the_flag_word_round_trips() {
    let file = test_fixtures::read("mp4/boxes/track-group-ster.mp4");
    let moov = moov_of(&file);
    let movie = MovieBox::parse(&moov).expect("moov parses");
    let track = &movie.tracks[0];
    let group = &track.trgr.as_ref().expect("trgr").groups()[0];
    assert_eq!(group.group_type(), *b"ster");
    assert_eq!(group.group_id(), 1);
    assert_eq!(
        group.left_view_flag(),
        None,
        "GPAC writes the 16-byte form without the flag word"
    );
    assert_eq!(
        track
            .trgr
            .as_ref()
            .expect("trgr")
            .try_to_bytes()
            .expect("bytes"),
        one_child(&children(&moov, *b"trak")[0], *b"trgr")
    );
}

#[test]
fn ssix_fixture_carries_the_documented_ranges() {
    let file = test_fixtures::read("mp4/boxes/ssix-sidx.mp4");
    // mp4dump: `[sidx] size=12+32` then `[ssix] size=8+20` for each segment.
    let mut ssix_boxes = Vec::new();
    let mut offset = 0usize;
    while let Some(found) = file[offset..]
        .windows(4)
        .position(|window| window == b"ssix")
    {
        let start = offset + found - 4;
        let size = u32::from_be_bytes(file[start..start + 4].try_into().expect("size")) as usize;
        ssix_boxes.push(file[start..start + size].to_vec());
        offset = start + size;
    }
    assert_eq!(ssix_boxes.len(), 6);
    let first = isobmff::SubsegmentIndexBox::parse(&ssix_boxes[0]).expect("ssix");
    assert_eq!(first.subsegments().len(), 1);
    let ranges = first.subsegments()[0].ranges();
    assert_eq!(ranges.len(), 2);
    assert_eq!((ranges[0].level(), ranges[0].range_size()), (1, 3236));
    assert_eq!((ranges[1].level(), ranges[1].range_size()), (2, 12184));
    assert_eq!(first.try_to_bytes().expect("ssix bytes"), ssix_boxes[0]);
}

#[test]
fn reader_surfaces_the_fragment_subsegment_index() {
    use isobmff::ReaderConfig;
    use isobmff::driver::{DriverStep, FileReader};

    let path = test_fixtures::path("mp4/boxes/ssix-sidx.mp4");
    let file = std::fs::File::open(path).expect("fixture opens");
    let mut reader = FileReader::new(file, ReaderConfig::default()).expect("reader");
    loop {
        match reader.step().expect("step") {
            DriverStep::Fragment(info) => {
                let ssix = info
                    .subsegment_index
                    .expect("ssix before the first fragment");
                assert_eq!(ssix.subsegments().len(), 1);
                assert_eq!(ssix.subsegments()[0].ranges()[0].range_size(), 3236);
                return;
            }
            DriverStep::Done => panic!("no fragment in the fixture"),
            _ => {}
        }
    }
}
