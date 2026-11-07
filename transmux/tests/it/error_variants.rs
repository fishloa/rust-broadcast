use broadcast_common::{Package, Unpackage};
use transmux::{AmfValue, FlvDemux, FlvError, MkvMux, ProtocolControl, RtmpError, TsDemux};

#[test]
fn a_short_control_body_names_the_message_type_and_both_lengths() {
    assert_eq!(
        ProtocolControl::parse(1, &[0, 0]).unwrap_err(),
        RtmpError::BadControlLength {
            msg_type: 1,
            need: 4,
            have: 2,
        }
    );
}

#[test]
fn an_unsupported_amf0_marker_is_reported() {
    assert_eq!(
        AmfValue::parse(&[0x07]).unwrap_err(),
        RtmpError::UnsupportedAmf0Marker(0x07)
    );
}

#[test]
fn an_amf0_string_beyond_the_16_bit_prefix_is_too_long() {
    let value = AmfValue::String("x".repeat(0x1_0000));
    let mut out = Vec::new();
    assert_eq!(
        value.write_into(&mut out).unwrap_err(),
        RtmpError::AmfStringTooLong { len: 0x1_0000 }
    );
}

#[test]
fn an_flv_with_no_tags_has_no_supported_track() {
    let header = [
        b'F', b'L', b'V', 0x01, 0x05, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00,
    ];
    assert_eq!(
        FlvDemux::new().unpackage(&header).unwrap_err(),
        FlvError::NoSupportedTrack
    );
}

#[test]
fn a_codec_matroska_cannot_carry_is_named_in_the_error() {
    let ts = test_fixtures::read("ts/legacy/mpeg2-mp2.ts");
    let media = TsDemux::new().unpackage(&ts).expect("demux");
    match MkvMux::new().package(&media).unwrap_err() {
        transmux::Error::UnsupportedMkvCodec { codec } => assert_eq!(codec, "MPEG-2 Video"),
        other => panic!("expected UnsupportedMkvCodec, got {other:?}"),
    }
}
