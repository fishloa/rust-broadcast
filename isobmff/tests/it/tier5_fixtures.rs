//! Real files carrying the tier 5 boxes of issue #4167.
//!
//! Each assertion names the line an independent tool's dump of the same file
//! prints, and every parsed box is re-serialized byte-exact from the file's
//! own bytes. `fixtures/mp4/boxes/PROVENANCE.md` records the command, tool
//! version and dump for each file.

use broadcast_core::{Parse, Serialize};
use isobmff::BoxIter;
use isobmff::MovieBox;
use isobmff::hint::{HintInformationChild, HintSampleEntry};
use isobmff::init_segment::{SampleEntryVariant, TrackBox};

fn top_level(file: &[u8], fourcc: [u8; 4]) -> Vec<u8> {
    for item in BoxIter::new(file) {
        let item = item.expect("top-level box");
        if item.four_cc() == fourcc {
            return item.bytes.to_vec();
        }
    }
    panic!("no top-level {fourcc:?}");
}

fn hint_tracks(movie: &MovieBox) -> Vec<&TrackBox> {
    movie
        .tracks
        .iter()
        .filter(|trak| {
            trak.mdia
                .as_ref()
                .and_then(|mdia| mdia.hdlr.as_ref())
                .is_some_and(|hdlr| &hdlr.handler_type == b"hint")
        })
        .collect()
}

fn rtp_entry(trak: &TrackBox) -> Option<isobmff::hint::RtpSampleEntry> {
    let stsd = trak
        .mdia
        .as_ref()?
        .minf
        .as_ref()?
        .stbl
        .as_ref()?
        .children
        .iter()
        .find_map(|child| match child {
            isobmff::init_segment::StblChild::Stsd(stsd) => Some(stsd.clone()),
            _ => None,
        })?;
    for entry in &stsd.entries {
        if let SampleEntryVariant::Hint(hint) = entry
            && let HintSampleEntry::Rtp(rtp) = hint.as_ref()
        {
            return Some(rtp.clone());
        }
    }
    None
}

#[test]
fn hint_offsets_fixture_reads_back() {
    let file = test_fixtures::read("mp4/boxes/hint-offsets.mp4");
    let moov = top_level(&file, *b"moov");
    let movie = MovieBox::parse(&moov).expect("moov parses");
    let hints = hint_tracks(&movie);
    assert_eq!(hints.len(), 2, "one hint track per media track");

    // MP4Box -disox: RTPHintSampleEntryBox MaxPacketSize="1450" / "1442",
    // RTPTimeScaleBox TimeScale="90000" / "44100",
    // TimeStampOffsetBox TimeStampOffset="0" and PacketSequenceOffsetBox
    // SeqNumOffset="0" in both entries.
    let video = rtp_entry(hints[0]).expect("video rtp entry");
    let audio = rtp_entry(hints[1]).expect("audio rtp entry");
    assert_eq!(video.format(), *b"rtp ");
    assert_eq!(video.hint_track_version(), 1);
    assert_eq!(video.highest_compatible_version(), 1);
    assert_eq!(video.max_packet_size(), 1450);
    assert_eq!(video.timescale(), Some(90_000));
    assert_eq!(video.time_offset(), Some(0));
    assert_eq!(video.sequence_offset(), Some(0));
    assert_eq!(audio.max_packet_size(), 1442);
    assert_eq!(audio.timescale(), Some(44_100));
    assert_eq!(audio.time_offset(), Some(0));
    assert_eq!(audio.sequence_offset(), Some(0));

    let stsd = hints[0]
        .mdia
        .as_ref()
        .and_then(|mdia| mdia.minf.as_ref())
        .and_then(|minf| minf.stbl.as_ref())
        .and_then(|stbl| {
            stbl.children.iter().find_map(|child| match child {
                isobmff::init_segment::StblChild::Stsd(stsd) => Some(stsd),
                _ => None,
            })
        })
        .expect("stsd");
    let SampleEntryVariant::Hint(hint) = &stsd.entries[0] else {
        panic!("expected the rtp entry");
    };
    let bytes = hint.try_to_bytes().expect("rtp entry bytes");
    assert_eq!(HintSampleEntry::parse_box(&bytes).expect("reparse"), **hint);
}

#[test]
fn hint_track_and_movie_sdp_read_back() {
    let file = test_fixtures::read("mp4/boxes/hint-offsets.mp4");
    let moov = top_level(&file, *b"moov");
    let movie = MovieBox::parse(&moov).expect("moov parses");
    let hints = hint_tracks(&movie);
    let track_udta = hints[0]
        .opaque
        .iter()
        .find(|child| child.box_type == *b"udta")
        .expect("hint track udta");
    let udta =
        isobmff::metadata::Udta::parse_body(&track_udta.data, &Default::default()).expect("udta");
    let hnti = udta
        .children
        .iter()
        .find_map(|child| match child {
            isobmff::metadata::UdtaChild::HintInformation(hint) => Some(hint),
            _ => None,
        })
        .expect("hnti");
    // MP4Box -disox: SDPBox Container="hnti", sdp text starts
    // "m=video 0 RTP/AVP 96".
    let sdp = hnti.sdp().expect("track sdp");
    assert!(sdp.sdp_text().starts_with(b"m=video 0 RTP/AVP 96"));
    let hnti_bytes = hnti.try_to_bytes().expect("hnti bytes");
    assert_eq!(
        isobmff::hint::HintInformationBox::parse(&hnti_bytes).expect("hnti reparse"),
        **hnti
    );

    let movie_udta = movie
        .opaque
        .iter()
        .find(|child| child.box_type == *b"udta")
        .expect("movie udta");
    let udta = isobmff::metadata::Udta::parse_body(&movie_udta.data, &Default::default())
        .expect("movie udta");
    let hnti = udta
        .children
        .iter()
        .find_map(|child| match child {
            isobmff::metadata::UdtaChild::HintInformation(hint) => Some(hint),
            _ => None,
        })
        .expect("movie hnti");
    // MP4Box -disox: RTPMovieHintInformationBox descriptionformat="sdp ".
    let rtp = hnti.rtp().expect("movie rtp");
    assert_eq!(rtp.description_format(), *b"sdp ");
    assert!(rtp.sdp_text().starts_with(b"b=AS:"));
    assert!(
        hnti.children()
            .iter()
            .any(|child| matches!(child, HintInformationChild::Rtp(_)))
    );
}

#[test]
fn hint_latm_audio_fixture_reads_back() {
    let file = test_fixtures::read("mp4/boxes/hint-latm.mp4");
    let moov = top_level(&file, *b"moov");
    let movie = MovieBox::parse(&moov).expect("moov parses");
    let hints = hint_tracks(&movie);
    assert_eq!(hints.len(), 1, "one audio hint track");
    let entry = rtp_entry(hints[0]).expect("audio rtp entry");
    // MP4Box -disox: MaxPacketSize="175", TimeScale="44100".
    assert_eq!(entry.max_packet_size(), 175);
    assert_eq!(entry.timescale(), Some(44_100));
    let bytes = entry.try_to_bytes().expect("entry bytes");
    assert_eq!(
        isobmff::hint::RtpSampleEntry::parse(&bytes).expect("reparse"),
        entry
    );
}

#[test]
fn hint_mtu_copy_fixture_reads_back() {
    let file = test_fixtures::read("mp4/boxes/hint-mtu-copy.mp4");
    let moov = top_level(&file, *b"moov");
    let movie = MovieBox::parse(&moov).expect("moov parses");
    let hints = hint_tracks(&movie);
    assert_eq!(hints.len(), 2);
    let video = rtp_entry(hints[0]).expect("video entry");
    let audio = rtp_entry(hints[1]).expect("audio entry");
    // MP4Box -disox: MaxPacketSize="800" for -mtu 800, and "794" carried by
    // the audio entry GPAC derived from it.
    assert_eq!(video.max_packet_size(), 800);
    assert_eq!(audio.max_packet_size(), 794);
    assert_eq!(video.timescale(), Some(90_000));
    assert_eq!(audio.timescale(), Some(44_100));
}

#[test]
fn bit_rate_box_from_a_lavf_file_reads_back() {
    let file = test_fixtures::read("mp4/h264-aac-prog.mp4");
    let tracks = reader_tracks(&file);
    let mut rates = Vec::new();
    for track in &tracks {
        for entry in &track.entries {
            if let Some(rate) = entry.bit_rate() {
                rates.push(rate);
            }
        }
    }
    // MP4Box -disox: video BufferSizeDB="0" avgBitRate="0" maxBitRate="46196";
    // audio avgBitRate="96800" maxBitRate="96800".
    assert_eq!(rates.len(), 2);
    assert_eq!(rates[0].buffer_size_db(), 0);
    assert_eq!(rates[0].avg_bitrate(), 0);
    assert_eq!(rates[0].max_bitrate(), 46_196);
    assert_eq!(rates[1].avg_bitrate(), 96_800);
    assert_eq!(rates[1].max_bitrate(), 96_800);
}

fn reader_tracks(file: &[u8]) -> Vec<isobmff::TrackInfo> {
    let total = u64::try_from(file.len()).expect("length");
    let config = isobmff::ReaderConfig::default().with_total_len(Some(total));
    let mut reader = isobmff::Reader::new(config);
    let mut tracks = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            isobmff::Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("range start");
                let end = usize::try_from(range.end()).expect("range end");
                reader
                    .supply(range.offset, &file[start..end])
                    .expect("supply");
            }
            isobmff::Step::Tracks(list) => tracks = list.to_vec(),
            isobmff::Step::Done => break,
            _ => {}
        }
    }
    tracks
}

#[test]
fn hint_track_facts_are_reachable_from_the_reader() {
    let file = test_fixtures::read("mp4/boxes/hint-offsets.mp4");
    let tracks = reader_tracks(&file);
    let hint = tracks
        .iter()
        .find(|track| &track.handler == b"hint")
        .expect("hint track");
    assert_eq!(hint.entries.len(), 1);
    assert!(matches!(
        hint.entries[0].hint(),
        Some(HintSampleEntry::Rtp(_))
    ));
    let hnti = hint.hint_information.as_ref().expect("hnti on the track");
    assert!(hnti.sdp().expect("sdp").sdp_text().starts_with(b"m=video"));
    assert!(hint.hint_statistics.is_none());
}

#[test]
fn fd_item_information_is_reachable_from_meta() {
    let file = test_fixtures::read("mp4/frag/av1.frag.mp4");
    let moov = top_level(&file, *b"moov");
    let movie = MovieBox::parse(&moov).expect("moov parses");
    for meta in movie
        .opaque
        .iter()
        .filter(|child| child.box_type == *b"meta")
    {
        let parsed =
            isobmff::metadata::Meta::parse_body(&meta.data, &Default::default()).expect("meta");
        assert!(parsed.fd_item_information().is_none());
    }
}
