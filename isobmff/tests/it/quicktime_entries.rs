//! QuickTime video and sound sample descriptions on real `.mov` files.
//!
//! Each file is one this workspace generated with `ffmpeg` (see
//! `fixtures/PROVENANCE.md`); the `MP4Box -diso` line quoted beside each case
//! is the independent oracle for the values asserted here, and every parsed
//! `stsd` is re-serialized byte for byte.

use broadcast_core::{Parse, Serialize};
use isobmff::box_types::BoxIter;
use isobmff::init_segment::{SampleDescriptionBox, SampleEntryVariant};
use test_fixtures as fixtures;

/// Split a `stsd`'s box bytes out of a whole `.mov`.
fn stsd_of(data: &[u8]) -> Vec<u8> {
    fn walk(data: &[u8], found: &mut Vec<u8>) {
        for item in BoxIter::new(data) {
            let Ok(item) = item else { return };
            match &item.four_cc() {
                b"stsd" => *found = item.bytes.to_vec(),
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => walk(item.body, found),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    walk(data, &mut found);
    found
}

fn entry(path: &str) -> SampleEntryVariant {
    let data = fixtures::read(path);
    let stsd = stsd_of(&data);
    let table = SampleDescriptionBox::parse(&stsd).expect("stsd parses");
    let back = table.try_to_bytes().expect("stsd serializes");
    assert_eq!(back, stsd, "{path}: stsd round trip");
    table.entries.into_iter().next().expect("one entry")
}

fn video(path: &str, code: [u8; 4], width: u16, height: u16) {
    match entry(path) {
        SampleEntryVariant::QuickTimeVideo(v) => {
            assert_eq!(v.codec_type(), code, "{path}");
            assert_eq!(v.visual().width, width, "{path}");
            assert_eq!(v.visual().height, height, "{path}");
        }
        other => panic!("{path}: {other:?}"),
    }
}

fn sound(path: &str, code: [u8; 4], channels: u16, rate: u32) {
    match entry(path) {
        SampleEntryVariant::QuickTimeSound(s) => {
            assert_eq!(s.codec_type, code, "{path}");
            assert_eq!(s.channelcount, channels, "{path}");
            assert_eq!(s.samplerate, rate, "{path}");
        }
        other => panic!("{path}: {other:?}"),
    }
}

#[test]
fn prores_profiles_are_the_standard_video_description() {
    // MP4Box -diso fixtures/mp4/quicktime/qt-prores-apch.mov:
    //   <ProResSampleEntryBox Size="112" Type="apch" ... Width="64" Height="48"
    //    BitDepth="24" CompressorName="Lavc62.28.102 prores_ks">
    for (file, code) in [
        ("mp4/quicktime/qt-prores-apco.mov", *b"apco"),
        ("mp4/quicktime/qt-prores-apcs.mov", *b"apcs"),
        ("mp4/quicktime/qt-prores-apcn.mov", *b"apcn"),
        ("mp4/quicktime/qt-prores-apch.mov", *b"apch"),
        ("mp4/quicktime/qt-prores-ap4h.mov", *b"ap4h"),
        ("mp4/quicktime/qt-prores-ap4x.mov", *b"ap4x"),
    ] {
        video(file, code, 64, 48);
    }
}

#[test]
fn mjpeg_and_svq1_are_the_standard_video_description() {
    // MP4Box -diso fixtures/mp4/quicktime/qt-mjpeg.mov:
    //   <MPEGVisualSampleDescriptionBox Size="112" Type="jpeg" ... Width="64" Height="48">
    // MP4Box -diso fixtures/mp4/quicktime/qt-svq1.mov:
    //   <VisualSampleDescriptionBox Type="SVQ1" ... Width="64" Height="48">
    video("mp4/quicktime/qt-mjpeg.mov", *b"jpeg", 64, 48);
    video("mp4/quicktime/qt-svq1.mov", *b"SVQ1", 64, 48);
}

#[test]
fn h263_is_the_standard_video_description() {
    // MP4Box -diso fixtures/mp4/quicktime/qt-h263.mov:
    //   <VisualSampleDescriptionBox Type="h263" ... Width="176" Height="144">
    video("mp4/quicktime/qt-h263.mov", *b"h263", 176, 144);
}

#[test]
fn pcm_sound_entries_carry_their_rate_and_channel_count() {
    // MP4Box -diso fixtures/mp4/quicktime/qt-pcm-s16le.mov:
    //   <AudioSampleDescriptionBox Type="sowt" SampleRate="8000" Channels="1"
    //    BitsPerSample="16">
    sound("mp4/quicktime/qt-pcm-s16le.mov", *b"sowt", 1, 8000 << 16);
    // MP4Box -diso fixtures/mp4/quicktime/qt-pcm-s24le.mov:
    //   <AudioSampleDescriptionBox Type="in24" Version="1" SampleRate="8000"
    //    Channels="1" BitsPerSample="16" ...>
    sound("mp4/quicktime/qt-pcm-s24le.mov", *b"in24", 1, 8000 << 16);
    match entry("mp4/quicktime/qt-pcm-s24le.mov") {
        SampleEntryVariant::QuickTimeSound(s) => assert_eq!(s.entry_version, 1),
        other => panic!("in24: {other:?}"),
    }
}
