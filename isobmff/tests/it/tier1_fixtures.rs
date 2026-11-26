//! Tier 1 boxes found in real files written by GPAC 26.07 (`MP4Box`), ffmpeg
//! 8.1.2 and Apple's `afconvert`, with the values cross-checked against
//! `mp4dump` (where it prints the box) or an independent bit-level read of the
//! file's bytes for the same file.
//!
//! `fixtures/mp4/boxes/PROVENANCE.md` records the source, the exact command and
//! the tool version for each file. Every typed box is serialized back and
//! compared byte-for-byte with the bytes in the fixture.

use broadcast_core::{Parse, Serialize};
use isobmff::metadata::{Udta, UdtaChild};
use isobmff::reader::TrackInfo;
use isobmff::{BoxIter, MovieBox, parse_box};

/// Offset of the first child box of an `AudioSampleEntryV0` inside the entry's
/// own bytes: the 8-byte box header plus the 28-byte tail (§12.2.3.2).
const AUDIO_ENTRY_CHILDREN_OFFSET: usize = 8 + 28;
/// Offset of `channelcount` inside an `AudioSampleEntryV0`'s own bytes: the
/// 8-byte box header plus the 16-byte offset within the tail (§12.2.3.2).
const AUDIO_ENTRY_CHANNELCOUNT_OFFSET: usize = 8 + 16;

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

#[test]
fn chnl_fixture_carries_both_wire_forms_ffmpeg_writes() {
    let file = test_fixtures::read("mp4/boxes/chnl-ipcm.mp4");
    let moov = moov_of(&file);
    let movie = MovieBox::parse(&moov).expect("moov parses");
    assert_eq!(movie.tracks.len(), 2);

    let mut layouts = Vec::new();
    let mut channel_counts = Vec::new();
    for track in &movie.tracks {
        let info = TrackInfo::from_trak(track).expect("track info");
        let entry = info.entries.first().expect("one stsd entry");
        assert_eq!(entry.fourcc, *b"ipcm");
        let chnl = BoxIter::new(&entry.body[AUDIO_ENTRY_CHILDREN_OFFSET..])
            .filter_map(|child| child.ok())
            .find(|child| child.four_cc() == *b"chnl")
            .expect("chnl inside the ipcm entry");
        let layout = isobmff::ChannelLayoutBox::parse(chnl.bytes).expect("chnl parses");
        assert_eq!(
            layout.try_to_bytes().expect("chnl bytes"),
            chnl.bytes,
            "chnl re-serializes byte-exact"
        );
        let count =
            &entry.body[AUDIO_ENTRY_CHANNELCOUNT_OFFSET..AUDIO_ENTRY_CHANNELCOUNT_OFFSET + 2];
        channel_counts.push(u16::from_be_bytes(count.try_into().expect("channelcount")));
        layouts.push(layout);
    }

    // Track 1 is stereo, so ffmpeg names the standard layout instead of listing
    // positions, and the omitted-channel map is present and empty.
    let stereo = &layouts[0];
    assert_eq!(stereo.version(), 0);
    assert!(stereo.is_channel_structured());
    assert!(!stereo.is_object_structured());
    assert_eq!(stereo.defined_layout(), Some(2));
    assert_eq!(stereo.omitted_channels_map(), Some(0));
    assert!(stereo.speaker_positions().is_empty());

    // Track 2 has no standard layout, so every position is carried explicitly,
    // and `layout_channel_count` is the sample entry's own channel count.
    let surround = &layouts[1];
    assert_eq!(surround.defined_layout(), Some(0));
    assert_eq!(surround.omitted_channels_map(), None);
    let positions: Vec<u8> = surround
        .speaker_positions()
        .iter()
        .map(|position| position.speaker_position())
        .collect();
    assert_eq!(positions, [0, 1, 2, 3, 8, 9]);
    assert_eq!(usize::from(channel_counts[1]), positions.len());
}

#[test]
fn tsel_fixture_is_the_switch_group_mp4box_writes() {
    let file = test_fixtures::read("mp4/boxes/tsel.mp4");
    let moov = moov_of(&file);
    let movie = MovieBox::parse(&moov).expect("moov parses");
    let traks = children(&moov, *b"trak");
    assert_eq!(movie.tracks.len(), 2);

    let selected = TrackInfo::from_trak(&movie.tracks[0]).expect("track 0");
    let tsel = selected.track_selection.expect("tsel on track 1");
    assert_eq!(tsel.switch_group(), 7);
    assert_eq!(tsel.attributes(), &[*b"cdec", *b"scsz"]);
    assert_eq!(
        tsel.try_to_bytes().expect("tsel bytes"),
        one_child(&one_child(&traks[0], *b"udta"), *b"tsel")
    );

    let other = TrackInfo::from_trak(&movie.tracks[1]).expect("track 1");
    assert!(
        other.track_selection.is_none(),
        "the second track joins no switch group"
    );
}

#[test]
fn ludt_fixture_carries_apples_track_loudness() {
    let file = test_fixtures::read("mp4/boxes/loudness.m4a");
    let moov = moov_of(&file);
    let movie = MovieBox::parse(&moov).expect("moov parses");
    assert_eq!(movie.tracks.len(), 1);

    let info = TrackInfo::from_trak(&movie.tracks[0]).expect("track info");
    let loudness = info.loudness.expect("ludt under trak/udta");
    assert!(loudness.album().is_empty(), "Apple writes only tlou");
    assert_eq!(loudness.track().len(), 1);

    let base = loudness.track()[0].base();
    assert_eq!(base.version(), 1);
    assert_eq!(base.entries().len(), 1);
    let entry = &base.entries()[0];
    assert_eq!(entry.eq_set_id(), Some(0));
    assert_eq!(entry.downmix_id(), 0);
    assert_eq!(entry.drc_set_id(), 0);
    assert_eq!(entry.sample_peak_level(), 1103);
    assert_eq!(entry.true_peak_level(), 1102);
    assert_eq!(entry.measurement_system_for_tp(), 2);
    assert_eq!(entry.reliability_for_tp(), 3);
    let measurements: Vec<(u8, u8, u8, u8)> = entry
        .measurements()
        .iter()
        .map(|measurement| {
            (
                measurement.method_definition(),
                measurement.method_value(),
                measurement.measurement_system(),
                measurement.reliability(),
            )
        })
        .collect();
    assert_eq!(measurements, [(1, 143, 2, 3), (4, 144, 0, 0)]);

    let udta = one_child(&children(&moov, *b"trak")[0], *b"udta");
    assert_eq!(
        loudness.try_to_bytes().expect("ludt bytes"),
        one_child(&udta, *b"ludt")
    );
    assert_eq!(
        loudness.track()[0].try_to_bytes().expect("tlou bytes"),
        one_child(&one_child(&udta, *b"ludt"), *b"tlou")
    );
}
