use broadcast_common::error::FieldOverflow;
use broadcast_common::timescale::Timescale;
use rtmp_runtime::RtmpError;
use test_harness::collect::Collected as Media;
use transmux::{Error, FlvError, MkvMuxStage, RtmpFlvError, RtmpMuxStage};

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
    let err = crate::mux_bytes::mux_result(&mut RtmpMuxStage::with_chunk_size(usize::MAX), &media)
        .unwrap_err();
    assert_eq!(err, RtmpFlvError::ChunkSizeTooLarge { size: usize::MAX });
}

#[test]
fn an_flv_with_no_tags_has_no_supported_track() {
    let header = [
        b'F', b'L', b'V', 0x01, 0x05, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00,
    ];
    assert_eq!(
        test_harness::collect::media_from_flv(&header).unwrap_err(),
        FlvError::NoSupportedTrack
    );
}

#[test]
fn a_codec_matroska_cannot_carry_is_named_in_the_error() {
    let ts = test_fixtures::read("ts/legacy/mpeg2-mp2.ts");
    let media = test_harness::collect::media_from_ts(&ts).expect("demux");
    let mut stage = MkvMuxStage::new(core::time::Duration::from_millis(1_000));
    match crate::mux_bytes::mux_result(&mut stage, &media).unwrap_err() {
        transmux::Error::UnsupportedMkvCodec { codec } => assert_eq!(codec, "MPEG-2 Video"),
        other => panic!("expected UnsupportedMkvCodec, got {other:?}"),
    }
}

#[test]
fn flv_error_display_is_exact_for_every_variant() {
    use std::error::Error as _;

    let cases = [
        (
            FlvError::BadSignature(*b"ABC"),
            "bad FLV signature: [41, 42, 43] (expected \"FLV\")",
        ),
        (
            FlvError::TagOverrun {
                offset: 13,
                need: 20,
                have: 4,
            },
            "FLV tag at offset 13 overruns buffer: need 20, have 4",
        ),
        (
            FlvError::NoSupportedTrack,
            "FLV carried no supported track (need AVC video or AAC audio)",
        ),
        (
            FlvError::HeaderTooLarge {
                declared: 99,
                max: 64,
            },
            "FLV header DataOffset 99 exceeds the maximum accepted 64 bytes",
        ),
        (
            FlvError::UnsupportedCodec { codec: "HEVC" },
            "codec HEVC has no FLV carriage in this crate (only AVC + AAC)",
        ),
        (
            FlvError::Codec(Error::BoxSizeUnderflow {
                size: 4,
                header_size: 8,
            }),
            "FLV codec config: box size 4 is smaller than header (8 bytes)",
        ),
        (
            FlvError::TimestampBackwards {
                previous: Some(90),
                next: None,
            },
            "FLV tag timestamp went backwards: None follows Some(90)",
        ),
        (
            FlvError::SampleDurationTooLarge {
                ticks: 5_000_000_000,
            },
            "FLV sample span 5000000000 ticks exceeds a 32-bit duration",
        ),
        (
            FlvError::Header(RtmpError::Malformed { what: "header" }),
            "FLV header: malformed header",
        ),
        (
            FlvError::Tag(RtmpError::Malformed { what: "tag" }),
            "FLV tag: malformed tag",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
        assert!(error.source().is_none(), "{text}");
    }
}

#[test]
fn rtmp_flv_error_display_is_exact_for_every_variant() {
    use std::error::Error as _;

    let cases = [
        (
            RtmpFlvError::Flv(FlvError::NoSupportedTrack),
            "RTMP FLV routing: FLV carried no supported track (need AVC video or AAC audio)",
        ),
        (
            RtmpFlvError::ChunkSizeTooLarge { size: 0 },
            "RTMP chunk size 0 is not in 1..=16777216",
        ),
        (
            RtmpFlvError::MessageTooLarge { len: 10, max: 4 },
            "RTMP message body 10 bytes exceeds the 4-byte message limit",
        ),
        (
            RtmpFlvError::Rtmp(RtmpError::Malformed { what: "chunk" }),
            "RTMP wire: malformed chunk",
        ),
        (
            RtmpFlvError::FieldOverflow(FieldOverflow {
                field: "chunkSize",
                value: 7,
                max: 3,
            }),
            "RTMP chunkSize = 7 exceeds the field maximum 3",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
        assert!(error.source().is_none(), "{text}");
    }
}
