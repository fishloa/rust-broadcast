//! DASH manifest gates: RFC 6381 MPEG codec strings, addressing labels and an
//! empty caller-supplied duration list (ISO/IEC 23009-1 §5.3.9.4.4).
#![cfg(feature = "std")]

use broadcast_common::Package;
use broadcast_common::Unpackage;
use isobmff::mp4esds::{DecoderConfigDescriptor, ESDescriptor, EsdsBox};
use transmux::dash::{Addressing, DashPackager, TrackSegments};
use transmux::{CodecConfig, MpegAudioLayer, TsDemux, rfc6381_codec_string};

fn esds(oti: u8) -> EsdsBox {
    EsdsBox {
        es_descriptor: ESDescriptor::new(
            1,
            0,
            Some(DecoderConfigDescriptor::new(
                oti, 0x04, false, 0, 0, 0, None,
            )),
            None,
        ),
    }
}

#[test]
fn mpeg_codec_strings_carry_the_esds_object_type_indication() {
    let video = CodecConfig::Mpeg2Video {
        esds: esds(0x61),
        width: 720,
        height: 576,
    };
    assert_eq!(rfc6381_codec_string(&video).expect("codec"), "mp4v.61");
    let other = CodecConfig::Mpeg2Video {
        esds: esds(0x60),
        width: 720,
        height: 576,
    };
    assert_eq!(rfc6381_codec_string(&other).expect("codec"), "mp4v.60");
    let audio = CodecConfig::MpegAudio {
        esds: esds(0x6B),
        layer: MpegAudioLayer::LayerII,
        channel_count: 2,
        sample_rate: 48_000,
        sample_size: 16,
    };
    assert_eq!(rfc6381_codec_string(&audio).expect("codec"), "mp4a.6B");
}

#[test]
fn addressing_labels_are_stable() {
    assert_eq!(Addressing::Number.name(), "number");
    assert_eq!(Addressing::Timeline.name(), "timeline");
}

#[test]
fn an_empty_duration_list_falls_back_to_the_total_duration() {
    let bytes = test_fixtures::read("ts/h264-aac.ts");
    let media = TsDemux::new().unpackage(bytes.as_slice()).expect("demux");
    let video_id = media
        .tracks
        .iter()
        .find(|t| t.config().is_video())
        .expect("video track")
        .spec
        .track_id;
    let mut packager = DashPackager {
        segments: vec![TrackSegments {
            track_id: video_id,
            durations: Vec::new(),
        }],
        ..DashPackager::default()
    };
    let mpd = packager
        .package(&media)
        .expect("an empty list must not index durs[0]");
    assert!(mpd.contains("<SegmentTemplate"));
}
