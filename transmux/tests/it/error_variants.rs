use broadcast_common::error::FieldOverflow;
use broadcast_common::timescale::Timescale;
use broadcast_common::{Package, Unpackage};
use transmux::{FlvDemux, FlvError, Media, MkvMux, RtmpFlvError, RtmpMux, TsDemux};

#[test]
fn rtmp_flv_error_display_names_the_failure_and_its_numbers() {
    assert_eq!(
        RtmpFlvError::Truncated {
            what: "x",
            need: 5,
            have: 2
        }
        .to_string(),
        "RTMP truncated while parsing x: need 5, have 2"
    );

    let chunk = RtmpFlvError::ChunkSizeTooLarge { size: usize::MAX }.to_string();
    assert!(chunk.contains("chunk size"), "{chunk}");
    assert!(chunk.contains(&usize::MAX.to_string()), "{chunk}");

    let message = RtmpFlvError::MessageTooLarge { len: 10, max: 4 }.to_string();
    assert!(message.contains("10"), "{message}");
    assert!(message.contains('4'), "{message}");

    let overflow = RtmpFlvError::FieldOverflow(FieldOverflow {
        field: "chunkSize",
        value: 7,
        max: 3,
    })
    .to_string();
    assert!(overflow.contains("chunkSize"), "{overflow}");
    assert!(overflow.contains('7'), "{overflow}");
    assert!(overflow.contains('3'), "{overflow}");
}

#[test]
fn an_rtmp_chunk_size_beyond_the_u32_field_is_reported_not_wrapped() {
    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("a non-zero timescale")
    }

    let media = Media::new(Vec::new(), timescale(1000));
    let err = RtmpMux::with_chunk_size(usize::MAX)
        .package(&media)
        .unwrap_err();
    assert_eq!(err, RtmpFlvError::ChunkSizeTooLarge { size: usize::MAX });
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
