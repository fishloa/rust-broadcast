//! FLV mux/demux boundary and hostile-input gates (Adobe FLV v10.1 Annex E).
//!
//! The muxer's `CompositionTime` is a 3-byte SI24 (§E.4.3.2) and the demuxer's
//! tag walk must survive every short/hostile shape at its exact boundary. The
//! assertions read literal wire bytes and literal duration/timestamp values,
//! never a value recomputed from the code under test.
#![cfg(feature = "std")]

use broadcast_common::timescale::Timescale;
use broadcast_common::{Package, Parse, Unpackage};
use transmux::{CodecConfig, FlvDemux, FlvError, FlvMux, Media, Sample, Track, TrackId, TrackSpec};

use rtmp_runtime::flv::{FlvHeader, FlvTag, TagType, tags};
use transmux::ir::DataIdentity;

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn source_media() -> Media {
    FlvDemux::new()
        .unpackage(test_fixtures::read("flv/av.flv").as_slice())
        .expect("demux the real av.flv fixture")
}

fn avc_config() -> (isobmff::AVCConfigurationBox, u16, u16) {
    let media = source_media();
    match media.tracks[0].config() {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => (config.clone(), *width, *height),
        other => panic!("fixture track 0 must be AVC, got {other:?}"),
    }
}

fn aac_config() -> (isobmff::EsdsBox, u16, u32) {
    let media = source_media();
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
        .expect("fixture must carry an AAC track");
    match track.config() {
        CodecConfig::Aac {
            esds,
            channel_count,
            sample_rate,
            ..
        } => (esds.clone(), *channel_count, *sample_rate),
        other => panic!("expected AAC, got {other:?}"),
    }
}

fn video_track(samples: Vec<Sample>) -> Track {
    let (config, width, height) = avc_config();
    Track::new(
        TrackSpec::new(
            TrackId::new(1),
            timescale(1000),
            CodecConfig::Avc {
                config,
                width,
                height,
            },
        ),
        samples,
    )
}

fn timed_video(pts_offset: i64, dts: i64, duration: u32, data: Vec<u8>) -> Sample {
    Sample::new(
        data,
        Some(dts),
        Some(dts + pts_offset),
        Some(duration),
        true,
    )
    .expect("valid timing")
}

fn tag_bodies(flv: &[u8]) -> Vec<FlvTag<'_>> {
    let header = FlvHeader::parse(flv).expect("valid FLV header");
    tags(flv, &header)
        .map(|tag| tag.expect("complete tag"))
        .collect()
}

fn video_nalu_bodies(flv: &[u8]) -> Vec<Vec<u8>> {
    tag_bodies(flv)
        .into_iter()
        .filter(|t| t.tag_type == TagType::Video && t.body.len() >= 5 && t.body[1] == 1)
        .map(|t| t.body.to_vec())
        .collect()
}

fn flv_prefix() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"FLV");
    out.push(1);
    out.push(0x05);
    out.extend_from_slice(&9u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out
}

fn push_tag(out: &mut Vec<u8>, tag_type: u8, timestamp: u32, body: &[u8]) {
    let start = out.len();
    out.push(tag_type);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    out.extend_from_slice(&timestamp.to_be_bytes()[1..4]);
    out.push((timestamp >> 24) as u8);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(body);
    let size = (out.len() - start) as u32;
    out.extend_from_slice(&size.to_be_bytes());
}

fn flv_with(tags: &[(u8, u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = flv_prefix();
    for (tag_type, timestamp, body) in tags {
        push_tag(&mut out, *tag_type, *timestamp, body);
    }
    out
}

fn sequence_body(tag_type: u8, packet_type: u8) -> Vec<u8> {
    let bytes = test_fixtures::read("flv/av.flv");
    let header = FlvHeader::parse(bytes.as_slice()).expect("fixture header");
    for tag in tags(bytes.as_slice(), &header) {
        let tag = tag.expect("complete tag");
        let wanted = match tag_type {
            9 => tag.body.first().is_some_and(|b| b & 0x0F == 7),
            _ => tag.body.first().is_some_and(|b| b >> 4 == 10),
        };
        if tag.tag_type == TagType::from_byte(tag_type)
            && wanted
            && tag.body.len() > 1
            && tag.body[1] == packet_type
        {
            return tag.body.to_vec();
        }
    }
    panic!("fixture has no sequence header for tag type {tag_type}");
}

#[test]
fn composition_time_is_written_as_the_exact_si24_bytes() {
    let offsets = [0i64, 1, 255, 256, 65_536, -1];
    let samples = offsets
        .iter()
        .enumerate()
        .map(|(i, &offset)| timed_video(offset, i as i64 * 1000, 1000, vec![0u8; 4]))
        .collect();
    let media = Media::new(vec![video_track(samples)], timescale(1000));
    let flv = FlvMux::new().package(&media).expect("package");
    let bodies = video_nalu_bodies(&flv);
    let expected: [[u8; 3]; 6] = [
        [0x00, 0x00, 0x00],
        [0x00, 0x00, 0x01],
        [0x00, 0x00, 0xFF],
        [0x00, 0x01, 0x00],
        [0x01, 0x00, 0x00],
        [0xFF, 0xFF, 0xFF],
    ];
    assert_eq!(bodies.len(), expected.len());
    for (body, want) in bodies.iter().zip(expected) {
        assert_eq!(&body[2..5], &want, "CompositionTime bytes");
    }
}

#[test]
fn composition_time_si24_bounds_are_inclusive() {
    let ok_max = timed_video(8_388_607, 0, 1000, vec![0u8; 4]);
    let media = Media::new(vec![video_track(vec![ok_max])], timescale(1000));
    let flv = FlvMux::new()
        .package(&media)
        .expect("SI24_MAX is representable");
    assert_eq!(&video_nalu_bodies(&flv)[0][2..5], &[0x7F, 0xFF, 0xFF]);

    let ok_min = timed_video(-8_388_608, 0, 1000, vec![0u8; 4]);
    let media = Media::new(vec![video_track(vec![ok_min])], timescale(1000));
    let flv = FlvMux::new()
        .package(&media)
        .expect("SI24_MIN is representable");
    assert_eq!(&video_nalu_bodies(&flv)[0][2..5], &[0x80, 0x00, 0x00]);

    for offset in [8_388_608i64, -8_388_609] {
        let sample = timed_video(offset, 0, 1000, vec![0u8; 4]);
        let media = Media::new(vec![video_track(vec![sample])], timescale(1000));
        let err = FlvMux::new()
            .package(&media)
            .expect_err("out-of-range CompositionTime must be rejected");
        assert_eq!(
            err,
            FlvError::Codec(transmux::Error::InvalidInput(
                "FLV CompositionTime does not fit the signed 24-bit field"
            )),
            "offset {offset}"
        );
    }
}

#[test]
fn the_tag_timestamp_31_bit_boundary_is_exact() {
    let max = u32::MAX;
    let at_max = vec![
        timed_video(0, 0, max, vec![0u8; 4]),
        timed_video(0, 0, 1000, vec![0u8; 4]),
    ];
    let media = Media::new(vec![video_track(at_max)], timescale(1000));
    let flv = FlvMux::new()
        .package(&media)
        .expect("u32::MAX ms is representable");
    let timestamps: Vec<u32> = tag_bodies(&flv)
        .into_iter()
        .filter(|t| t.tag_type == TagType::Video && t.body[1] == 1)
        .map(|t| t.timestamp)
        .collect();
    assert_eq!(timestamps, vec![0, max]);

    let past_max = vec![
        timed_video(0, 0, max, vec![0u8; 4]),
        timed_video(0, 0, 1, vec![0u8; 4]),
        timed_video(0, 0, 1, vec![0u8; 4]),
    ];
    let media = Media::new(vec![video_track(past_max)], timescale(1000));
    let err = FlvMux::new()
        .package(&media)
        .expect_err("u32::MAX + 1 ms must be rejected");
    assert_eq!(
        err,
        FlvError::Codec(transmux::Error::InvalidInput(
            "FLV tag timestamp does not fit the 32-bit millisecond clock"
        ))
    );
}

#[test]
fn audio_sound_type_follows_the_channel_count() {
    let (esds, _, sample_rate) = aac_config();
    for (channel_count, expected_low_bit) in [(1u16, 0u8), (2, 1)] {
        let track = Track::new(
            TrackSpec::new(
                TrackId::new(2),
                timescale(1000),
                CodecConfig::Aac {
                    esds: esds.clone(),
                    channel_count,
                    sample_rate,
                    sample_size: 16,
                },
            ),
            vec![Sample::new(vec![0u8; 4], Some(0), Some(0), Some(1000), true).unwrap()],
        );
        let media = Media::new(vec![track], timescale(1000));
        let payloads = transmux::flv_sequence_header_payloads(&media).expect("sequence headers");
        let audio = payloads
            .iter()
            .find(|p| p.kind == transmux::FlvPayloadKind::Audio)
            .expect("audio sequence header");
        assert_eq!(
            audio.body[0] & 1,
            expected_low_bit,
            "SoundType for {channel_count} channels"
        );
    }
}

#[test]
fn locate_av_tracks_picks_the_first_avc_and_aac_track() {
    let (avc, width, height) = avc_config();
    let (esds, channel_count, sample_rate) = aac_config();
    let video_spec = |id: u32| {
        TrackSpec::new(
            TrackId::new(id),
            timescale(1000),
            CodecConfig::Avc {
                config: avc.clone(),
                width,
                height,
            },
        )
    };
    let audio_spec = |id: u32| {
        TrackSpec::new(
            TrackId::new(id),
            timescale(1000),
            CodecConfig::Aac {
                esds: esds.clone(),
                channel_count,
                sample_rate,
                sample_size: 16,
            },
        )
    };
    let media = Media::new(
        vec![
            Track::new(
                video_spec(1),
                vec![Sample::new(b"FIRST".to_vec(), Some(0), Some(0), Some(1000), true).unwrap()],
            ),
            Track::new(
                video_spec(2),
                vec![Sample::new(b"SECOND".to_vec(), Some(0), Some(0), Some(1000), true).unwrap()],
            ),
            Track::new(
                audio_spec(3),
                vec![Sample::new(b"AA".to_vec(), Some(0), Some(0), Some(1000), true).unwrap()],
            ),
            Track::new(
                audio_spec(4),
                vec![Sample::new(b"BB".to_vec(), Some(0), Some(0), Some(1000), true).unwrap()],
            ),
        ],
        timescale(1000),
    );
    let payloads = transmux::flv_frame_payloads(&media).expect("frame payloads");
    let videos: Vec<_> = payloads
        .iter()
        .filter(|p| p.kind == transmux::FlvPayloadKind::Video)
        .collect();
    let audios: Vec<_> = payloads
        .iter()
        .filter(|p| p.kind == transmux::FlvPayloadKind::Audio)
        .collect();
    assert_eq!(videos.len(), 1, "only one video track may be carried");
    assert_eq!(&videos[0].body[5..], b"FIRST");
    assert_eq!(audios.len(), 1, "only one audio track may be carried");
    assert_eq!(&audios[0].body[2..], b"AA");
}

#[test]
fn an_unsupported_codec_is_named_in_the_mux_error() {
    let track = Track::new(
        TrackSpec::new(
            TrackId::new(1),
            timescale(1000),
            CodecConfig::Ac3 {
                config: isobmff::Ac3SpecificBox {
                    fscod: 0,
                    bsid: 8,
                    bsmod: 0,
                    acmod: 2,
                    lfeon: false,
                    bit_rate_code: 0,
                },
                channel_count: 2,
                sample_rate: 48_000,
                sample_size: 16,
            },
        ),
        vec![Sample::new(vec![0u8; 4], Some(0), Some(0), Some(1000), true).unwrap()],
    );
    let media = Media::new(vec![track], timescale(1000));
    assert_eq!(
        FlvMux::new().package(&media).unwrap_err(),
        FlvError::UnsupportedCodec { codec: "AC-3" }
    );
}

fn header_short_read(len: usize) -> FlvError {
    FlvDemux::new()
        .unpackage(&vec![0u8; len])
        .expect_err("a header this short cannot demux")
}

#[test]
fn a_header_shorter_than_nine_bytes_is_a_header_short_read() {
    for len in 0..9usize {
        let FlvError::Codec(transmux::Error::BufferTooShort(short)) = header_short_read(len) else {
            panic!("len {len}: wrong variant");
        };
        assert_eq!(
            short,
            broadcast_common::error::BufferTooShort {
                need: 9,
                have: len,
                what: "FLV header",
            },
            "len {len}"
        );
    }
}

#[test]
fn a_nine_byte_header_alone_is_a_tag_short_read() {
    let header = &flv_prefix()[..9];
    let FlvError::Codec(transmux::Error::BufferTooShort(short)) =
        FlvDemux::new().unpackage(header).expect_err("no tag")
    else {
        panic!("wrong variant");
    };
    assert_eq!(short.need, 13);
    assert_eq!(short.have, 9);
    assert_eq!(short.what, "FLV header");
}

#[test]
fn a_bad_signature_is_reported_with_its_bytes() {
    let mut bytes = flv_prefix();
    bytes[..3].copy_from_slice(b"XYZ");
    assert_eq!(
        FlvDemux::new().unpackage(&bytes).unwrap_err(),
        FlvError::BadSignature(*b"XYZ")
    );
}

#[test]
fn a_header_data_offset_past_the_buffer_is_a_short_read() {
    let mut bytes = flv_prefix();
    bytes[5..9].copy_from_slice(&100u32.to_be_bytes());
    let FlvError::Codec(transmux::Error::BufferTooShort(short)) = FlvDemux::new()
        .unpackage(&bytes)
        .expect_err("DataOffset past the buffer")
    else {
        panic!("wrong variant");
    };
    assert_eq!(short.need, 100);
    assert_eq!(short.have, 13);
    assert_eq!(short.what, "FLV tag body");
}

#[test]
fn a_header_data_offset_below_the_header_is_named() {
    let mut bytes = flv_prefix();
    bytes[5..9].copy_from_slice(&4u32.to_be_bytes());
    assert_eq!(
        FlvDemux::new().unpackage(&bytes).unwrap_err(),
        FlvError::Codec(transmux::Error::InvalidInput(
            "FLV header DataOffset below the header"
        ))
    );
}

#[test]
fn an_incomplete_first_tag_reports_the_overrun() {
    let mut bytes = flv_prefix();
    bytes.push(9);
    bytes.extend_from_slice(&[0x00, 0x03, 0xE8]);
    bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0]);
    let FlvError::Codec(transmux::Error::BufferTooShort(short)) = FlvDemux::new()
        .unpackage(&bytes)
        .expect_err("incomplete first tag")
    else {
        panic!("wrong variant");
    };
    assert_eq!(short.what, "FLV tag body");
    assert!(
        short.need > short.have,
        "need {} have {}",
        short.need,
        short.have
    );
}

#[test]
fn a_video_tag_shorter_than_its_header_is_skipped() {
    for body in [Vec::new(), vec![0x17], vec![0x17, 0x01, 0x00, 0x00]] {
        let flv = flv_with(&[(9, 0, body.clone())]);
        assert_eq!(
            FlvDemux::new().unpackage(&flv).unwrap_err(),
            FlvError::NoSupportedTrack,
            "body {body:?}"
        );
    }
}

#[test]
fn a_five_byte_video_tag_carries_an_empty_access_unit() {
    let flv = flv_with(&[
        (9, 0, sequence_body(9, 0)),
        (9, 0, vec![0x17, 0x01, 0x00, 0x00, 0x00]),
    ]);
    let media = FlvDemux::new().unpackage(&flv).expect("5-byte NALU tag");
    assert_eq!(media.tracks.len(), 1);
    assert_eq!(media.tracks[0].samples.len(), 1);
    assert!(media.tracks[0].samples[0].data.is_empty());
}

#[test]
fn an_audio_tag_shorter_than_two_bytes_is_skipped() {
    let flv = flv_with(&[(8, 0, vec![0xAF])]);
    assert_eq!(
        FlvDemux::new().unpackage(&flv).unwrap_err(),
        FlvError::NoSupportedTrack
    );
}

#[test]
fn a_two_byte_audio_tag_carries_an_empty_access_unit() {
    let mut flv = test_fixtures::read("flv/av.flv");
    let before = FlvDemux::new()
        .unpackage(flv.as_slice())
        .expect("real fixture")
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
        .expect("audio track")
        .samples
        .len();
    push_tag(&mut flv, 8, 100_000, &[0xAF, 0x01]);
    let media = FlvDemux::new().unpackage(&flv).expect("2-byte audio tag");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
        .expect("audio track");
    assert_eq!(track.samples.len(), before + 1);
    assert!(track.samples.last().expect("last").data.is_empty());
}

#[test]
fn a_second_avc_sequence_header_does_not_replace_the_first() {
    let first = sequence_body(9, 0);
    let mut second = first.clone();
    second[6] = first[6] ^ 0x01;
    let nalu = vec![0x17, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x09];
    let flv = flv_with(&[(9, 0, first.clone()), (9, 0, second), (9, 0, nalu)]);
    let media = FlvDemux::new()
        .unpackage(&flv)
        .expect("two sequence headers");
    let CodecConfig::Avc { config, .. } = media.tracks[0].config() else {
        panic!("track 0 must be AVC");
    };
    assert_eq!(config.config.profile_indication, first[6]);
}

#[test]
fn an_empty_avc_sequence_header_is_skipped_without_damage() {
    let flv = flv_with(&[
        (9, 0, vec![0x17, 0x00, 0x00, 0x00, 0x00]),
        (9, 0, sequence_body(9, 0)),
        (
            9,
            0,
            vec![0x17, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x09],
        ),
    ]);
    let media = FlvDemux::new()
        .unpackage(&flv)
        .expect("empty then real header");
    assert!(
        media.degraded.is_empty(),
        "an empty header is skipped, not damage"
    );
    assert_eq!(media.tracks.len(), 1);
}

#[test]
fn the_last_sample_repeats_the_second_to_last_forward_delta() {
    let dts = [0u32, 10, 30, 60, 100, 150];
    let mut tags = vec![(9u8, 0u32, sequence_body(9, 0))];
    for ts in dts {
        tags.push((
            9,
            ts,
            vec![0x17, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x09],
        ));
    }
    let media = FlvDemux::new()
        .unpackage(&flv_with(&tags))
        .expect("six samples");
    let durations: Vec<u32> = media.tracks[0]
        .samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0))
        .collect();
    assert_eq!(durations, vec![10, 20, 30, 40, 50, 50]);
}

// ---------------------------------------------------------------------------
// The classifier, its labels, and the onMetaData geometry it drives
// ---------------------------------------------------------------------------

/// Read the `width` and `height` numbers out of an AMF0 `onMetaData` body
/// (`String` name, then an ECMA Array of properties, AMF0 §2.4/§2.10) —
/// decoded here, on the wire format, so the assertion does not lean on the
/// encoder it is checking.
fn metadata_width_height(body: &[u8]) -> (f64, f64) {
    assert_eq!(body[0], 0x02, "the name is an AMF0 string");
    let name_len = usize::from(u16::from_be_bytes([body[1], body[2]]));
    let mut at = 3 + name_len;
    assert_eq!(body[at], 0x08, "onMetaData must carry an ECMA Array");
    at += 1 + 4;
    let mut found: [Option<f64>; 2] = [None, None];
    while at + 2 <= body.len() && body[at..at + 2] != [0, 0] {
        let key_len = usize::from(u16::from_be_bytes([body[at], body[at + 1]]));
        at += 2;
        let key = std::str::from_utf8(&body[at..at + key_len]).expect("utf8 key");
        at += key_len;
        match body[at] {
            0x00 => {
                let raw: [u8; 8] = body[at + 1..at + 9].try_into().expect("8 bytes");
                let value = f64::from_be_bytes(raw);
                if key == "width" {
                    found[0] = Some(value);
                } else if key == "height" {
                    found[1] = Some(value);
                }
                at += 9;
            }
            0x02 => {
                let len = usize::from(u16::from_be_bytes([body[at + 1], body[at + 2]]));
                at += 3 + len;
            }
            other => panic!("unexpected AMF0 type 0x{other:02X} for {key}"),
        }
    }
    (
        found[0].expect("onMetaData carries width"),
        found[1].expect("onMetaData carries height"),
    )
}

/// `onMetaData` advertises the AVC track's real geometry, not 0x0.
#[test]
fn on_metadata_body_carries_the_avc_track_geometry() {
    let (config, _, _) = avc_config();
    let track = Track::new(
        TrackSpec::new(
            TrackId::new(1),
            timescale(1000),
            CodecConfig::Avc {
                config,
                width: 1280,
                height: 720,
            },
        ),
        Vec::new(),
    );
    let media = Media::new(vec![track], timescale(1000));
    let body = transmux::on_metadata_body(&media, Some(1.0)).expect("metadata body");
    assert_eq!(metadata_width_height(&body), (1280.0, 720.0));
}

/// The public classifier decides which tracks an FLV/RTMP writer carries: AVC
/// is video, AAC is audio, and anything else has no FLV tag.
#[test]
fn flv_track_kind_classifies_exactly_the_carryable_codecs() {
    let (config, width, height) = avc_config();
    let avc = CodecConfig::Avc {
        config,
        width,
        height,
    };
    assert_eq!(
        transmux::flv_track_kind(&avc),
        Some(transmux::FlvPayloadKind::Video)
    );

    let (esds, channel_count, sample_rate) = aac_config();
    let aac = CodecConfig::Aac {
        esds,
        channel_count,
        sample_rate,
        sample_size: 16,
    };
    assert_eq!(
        transmux::flv_track_kind(&aac),
        Some(transmux::FlvPayloadKind::Audio)
    );

    let data = CodecConfig::Data {
        identity: DataIdentity::Ts {
            stream_type: 0x86,
            descriptors: Vec::new(),
            carriage: transmux::ir::DataCarriage::Sections,
        },
    };
    assert_eq!(transmux::flv_track_kind(&data), None);
}

/// The #204 label pair: the spec token, and `Display` equal to it.
#[test]
fn flv_payload_kind_labels_are_the_tag_tokens() {
    use transmux::FlvPayloadKind::{Audio, Video};
    assert_eq!(Video.name(), "video");
    assert_eq!(Audio.name(), "audio");
    assert_eq!(Video.to_string(), "video");
    assert_eq!(Audio.to_string(), "audio");
}
