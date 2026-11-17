//! AV3A (`av3a` feature) AATF frame header edge cases: every header shape's
//! exact bit budget, the optional-field presence rules, byte alignment, and
//! the spec labels (T/AI 109.7 Annex A, Tables A.4/A.6).
//!
//! Every frame here is built by parsing a hand-assembled vector (the header
//! types are `#[non_exhaustive]`), then serialized back byte-for-byte.

#![cfg(feature = "av3a")]

use rtp_packet::av3a::{AatfFrame, AatfFrameHeader, CodecNnId, GaContent};

fn round_trip(bytes: &[u8]) -> AatfFrame<'_> {
    let frame = AatfFrame::parse(bytes).expect("parse spec-derived header vector");
    let mut out = vec![0u8; frame.serialized_len()];
    let written = frame.serialize_into(&mut out).unwrap();
    assert_eq!(written, frame.serialized_len());
    assert_eq!(&out[..written], bytes, "byte-exact serialization");
    frame
}

#[test]
fn general_high_header_with_nonzero_alignment_bits_round_trips() {
    // 62 header bits then two `byte_alignment` bits (value 2), so the 8-byte
    // header's last byte ends in `...10`.
    let bytes = [0xFF, 0xF0, 0x19, 0x12, 0x34, 0xAB, 0xAB, 0x66];
    let frame = round_trip(&bytes);
    assert_eq!(frame.alignment, 2);
    assert_eq!(frame.serialized_len(), 8, "62 header bits occupy 8 bytes");
    assert_eq!(frame.header.name(), "general high-rate audio coding");
    let AatfFrameHeader::GeneralHigh(h) = frame.header else {
        panic!("audio_codec_id 0");
    };
    assert_eq!(h.coding_profile, 1);
    assert_eq!(h.sampling_frequency_index, 0x9);
    assert_eq!(h.raw_frame_length, 0x1234);
    assert_eq!(h.aatf_crc, 0xAB);
    assert_eq!(h.channel_number_index, 0x55);
    assert_eq!(h.resolution, 2);
    assert_eq!(h.bitrate_index, 0xC);
    assert!(h.bitstream_type);
}

#[test]
fn lossless_header_without_an_explicit_frequency_round_trips() {
    // sampling_frequency_index 3 != 0xF, so the 24-bit field is absent and
    // the header is exactly 48 bits: six bytes with no alignment padding.
    let bytes = [0xFF, 0xF1, 0x23, 0x12, 0x34, 0x56];
    let frame = round_trip(&bytes);
    assert_eq!(frame.alignment, 0);
    assert_eq!(frame.serialized_len(), 6);
    assert_eq!(frame.header.name(), "lossless audio coding");
    let AatfFrameHeader::Lossless(l) = frame.header else {
        panic!("audio_codec_id 1");
    };
    assert_eq!(l.coding_profile, 2);
    assert_eq!(l.sampling_frequency_index, 3);
    assert_eq!(l.sampling_frequency, None);
    assert_eq!(l.raw_frame_length, 0x1234);
    assert_eq!(l.aatf_crc, 0x56);
}

#[test]
fn general_full_objects_only_header_has_thirteen_content_bits() {
    // 58 header bits: 17 prefix + 3 nn_type + 3 coding_profile + 4 sfi + 8
    // aatf crc + 13 content (2 soundBedType + 7 object_channel_number + 4
    // bitrate_index_per_channel) + 2 resolution + 8 frame crc, padded to 8
    // bytes with six alignment bits (value 0).
    let bytes = [0xFF, 0xF2, 0x12, 0x4B, 0x40, 0x75, 0xB0, 0xC0];
    let frame = round_trip(&bytes);
    assert_eq!(frame.serialized_len(), 8);
    let AatfFrameHeader::GeneralFull(g) = frame.header else {
        panic!("audio_codec_id 2");
    };
    assert_eq!(g.nn_type, 1);
    assert_eq!(g.coding_profile, 1);
    assert_eq!(g.sampling_frequency_index, 2);
    assert_eq!(g.aatf_crc, 0x5A);
    assert_eq!(
        g.content,
        GaContent::ObjectsOnly {
            object_channel_number: 7,
            bitrate_index_per_channel: 5,
        }
    );
    assert_eq!(g.resolution, 2);
    assert_eq!(g.bitrate_index, None);
    assert_eq!(g.frame_crc, 0xC3);
    assert_eq!(g.content.name(), "objects only");

    let mut out = vec![0u8; frame.serialized_len()];
    frame.serialize_into(&mut out).unwrap();
    assert_eq!(out.len(), 8, "58 header bits rounded up to 8 bytes");
}

#[test]
fn every_general_full_content_shape_is_labelled_with_its_spec_token() {
    let cases: [(&[u8], GaContent, &str); 6] = [
        (
            &[0xFF, 0xF2, 0x00, 0x00, 0x00, 0x42, 0x00, 0x00],
            GaContent::Channel {
                channel_number_index: 1,
            },
            "channel signal",
        ),
        (
            &[0xFF, 0xF2, 0x02, 0x00, 0x00, 0x75, 0x00, 0x00],
            GaContent::ObjectsOnly {
                object_channel_number: 7,
                bitrate_index_per_channel: 5,
            },
            "objects only",
        ),
        (
            &[0xFF, 0xF2, 0x02, 0x00, 0x08, 0x32, 0x08, 0xC0, 0x00],
            GaContent::ChannelAndObjects {
                channel_number_index: 3,
                bitrate_index: 2,
                object_channel_number: 4,
                bitrate_index_per_channel: 6,
            },
            "channel signal and objects",
        ),
        (
            &[0xFF, 0xF2, 0x02, 0x00, 0x10, 0x00],
            GaContent::ReservedSoundBed(2),
            "reserved soundBedType",
        ),
        (
            &[0xFF, 0xF2, 0x04, 0x00, 0x04, 0x20, 0x00],
            GaContent::Hoa { order: 2 },
            "HOA signal",
        ),
        (
            &[0xFF, 0xF2, 0x06, 0x00, 0x00, 0x00, 0x00],
            GaContent::ReservedProfile(3),
            "reserved coding_profile",
        ),
    ];
    for (bytes, expected, label) in cases {
        let frame = round_trip(bytes);
        let AatfFrameHeader::GeneralFull(g) = frame.header else {
            panic!("audio_codec_id 2");
        };
        assert_eq!(g.content, expected);
        assert_eq!(g.content.name(), label);
        assert_eq!(std::format!("{}", g.content), label);
    }
}

#[test]
fn header_labels_are_the_spec_tokens() {
    let general_high = round_trip(&[0xFF, 0xF0, 0x19, 0x12, 0x34, 0xAB, 0xAB, 0x66]);
    assert_eq!(general_high.header.name(), "general high-rate audio coding");
    assert_eq!(
        std::format!("{}", general_high.header),
        "general high-rate audio coding"
    );

    let lossless = round_trip(&[0xFF, 0xF1, 0x23, 0x12, 0x34, 0x56]);
    assert_eq!(lossless.header.name(), "lossless audio coding");
    assert_eq!(std::format!("{}", lossless.header), "lossless audio coding");

    let general_full = round_trip(&[0xFF, 0xF2, 0x12, 0x4B, 0x40, 0x75, 0xB0, 0xC0]);
    assert_eq!(general_full.header.name(), "general full-rate audio coding");
    assert_eq!(
        std::format!("{}", general_full.header),
        "general full-rate audio coding"
    );
}

#[test]
fn codec_nn_id_packs_the_two_bytes() {
    assert_eq!(
        CodecNnId {
            audio_codec_id: 0x01,
            nn_type: 0x02,
        }
        .to_u16(),
        0x0102
    );
    assert_eq!(CodecNnId::from_u16(0xABCD).audio_codec_id, 0xAB);
    assert_eq!(CodecNnId::from_u16(0xABCD).nn_type, 0xCD);
}

#[test]
fn objects_only_header_with_nonzero_alignment_round_trips() {
    // 58 header bits, then six `byte_alignment` bits (0b101010) fill the
    // 8-byte header.
    let bytes = [0xFF, 0xF2, 0x12, 0x4B, 0x40, 0x75, 0xB1, 0xEA];
    let frame = round_trip(&bytes);
    assert_eq!(frame.alignment, 0b10_1010);
    assert_eq!(frame.serialized_len(), 8);
    let AatfFrameHeader::GeneralFull(g) = frame.header else {
        panic!("audio_codec_id 2");
    };
    assert_eq!(
        g.content,
        GaContent::ObjectsOnly {
            object_channel_number: 7,
            bitrate_index_per_channel: 5,
        }
    );
}
