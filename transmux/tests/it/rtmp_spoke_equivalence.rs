//! The RTMP spoke is FLV carried over an RTMP chunk stream (Adobe FLV v10.1
//! Annex E §E.4.2 / §E.4.3, Adobe RTMP 1.0 §5.3): an Audio/Video message body
//! IS an FLV tag body. So *both* packaging paths must reproduce the same
//! [`Media`] IR:
//!
//! - FLV: [`FlvMux`] → bytes → [`FlvDemux`]
//! - RTMP: [`FlvMux`] → tags → RTMP messages → chunks → [`RtmpDemux`]
//!
//! The IR alone cannot see a dropped script (`onMetaData`) message — the
//! FLV demuxer lifts no track from it — so the chunk stream is also decoded
//! back to its messages and compared against the FLV tags the FLV path
//! writes, which is the only thing that would notice.

use broadcast_common::{Package, Parse, Unpackage};
use rtmp_runtime::chunk::ChunkAssembler;
use rtmp_runtime::message::{ProtocolControl, msg_type};
use transmux::{FlvDemux, FlvMux, Media, RtmpDemux, RtmpMux};

const FIXTURES: &[&str] = &["flv/av.flv", "flv/aac-7-1.flv"];

const FLV_TAG_AUDIO: u8 = msg_type::AUDIO;
const FLV_TAG_VIDEO: u8 = msg_type::VIDEO;

/// Chunk sizes the spoke must round-trip: the §5.3 default, sizes that make
/// small messages span several chunks (1, 192), a size larger than any message
/// (4096), a U16-sized chunk, and the largest size the chunk writer adopts.
const CHUNK_SIZES: &[usize] = &[
    1,
    128,
    192,
    4096,
    65_536,
    rtmp_runtime::chunk::MAX_CHUNK_SIZE as usize,
];

fn track_summary(media: &Media) -> Vec<(String, usize, Vec<usize>)> {
    media
        .tracks
        .iter()
        .map(|t| {
            (
                format!("{:?}", t.spec.config),
                t.samples.len(),
                t.samples.iter().map(|s| s.data.len()).collect(),
            )
        })
        .collect()
}

/// `(tag type, timestamp, body)` for every tag an FLV byte stream carries,
/// walked with `rtmp-runtime`'s strict tag walker (no hand-rolled framing).
fn flv_tags(flv: &[u8]) -> Vec<(u8, u32, Vec<u8>)> {
    let header = <rtmp_runtime::flv::FlvHeader<'_> as Parse>::parse(flv).expect("FLV header");
    rtmp_runtime::flv::tags(flv, &header)
        .map(|tag| tag.map(|tag| (tag.tag_type.to_byte(), tag.timestamp, tag.body.to_vec())))
        .collect::<Result<Vec<_>, _>>()
        .expect("FLV tags walk")
}

/// The same tags as the RTMP messages the chunk stream must carry.
fn messages_of(tags: &[(u8, u32, Vec<u8>)]) -> Vec<(u8, u32, Vec<u8>)> {
    tags.iter()
        .map(|(tag_type, timestamp, body)| {
            let message_type_id = match *tag_type {
                FLV_TAG_AUDIO => msg_type::AUDIO,
                FLV_TAG_VIDEO => msg_type::VIDEO,
                _ => msg_type::DATA_AMF0,
            };
            (message_type_id, *timestamp, body.clone())
        })
        .collect()
}

fn sample_bytes(media: &Media) -> Vec<Vec<Vec<u8>>> {
    media
        .tracks
        .iter()
        .map(|t| t.samples.iter().map(|s| s.data.to_vec()).collect())
        .collect()
}

/// The IR both spokes must reproduce. Taken from the fixture's own demux and
/// then put through one FLV mux/demux, because [`FlvMux`] synthesises exactly
/// one `onMetaData` tag while a capture may carry several: the second demux
/// drops that script-only difference from the IR, so the comparison isolates
/// what the RTMP chunking round trip has to preserve.
fn normalised_media(bytes: &[u8]) -> Media {
    let media = FlvDemux::new().unpackage(bytes).expect("fixture FLV → IR");
    let flv = FlvMux::new().package(&media).expect("IR → FLV");
    FlvDemux::new().unpackage(&flv).expect("FLV → IR")
}

#[test]
fn the_rtmp_spoke_carries_the_same_media_as_the_flv_spoke() {
    for rel in FIXTURES {
        let bytes = test_fixtures::read(rel);
        let ground_truth = normalised_media(bytes.as_slice());
        assert!(
            !ground_truth.tracks.is_empty(),
            "{rel}: fixture must have at least one track"
        );
        let flv_bytes = FlvMux::new().package(&ground_truth).expect("IR to FLV");
        let expected = messages_of(&flv_tags(&flv_bytes));
        assert!(
            expected.iter().any(|(t, ..)| *t == msg_type::DATA_AMF0),
            "{rel}: the FLV path writes an onMetaData tag to compare against"
        );

        for &chunk_size in CHUNK_SIZES {
            let wire = RtmpMux::with_chunk_size(chunk_size)
                .package(&ground_truth)
                .unwrap_or_else(|e| panic!("{rel} at chunk size {chunk_size}: IR → RTMP: {e}"));
            assert!(
                !wire.is_empty(),
                "{rel}: the RTMP wire is non-empty at chunk size {chunk_size}"
            );
            let carried: Vec<(u8, u32, Vec<u8>)> = {
                let mut assembler = ChunkAssembler::new();
                assembler.feed(&wire);
                let mut out = Vec::new();
                while let Some(m) = assembler.next_message().unwrap_or_else(|e| {
                    panic!("{rel} at chunk size {chunk_size}: chunk stream: {e}")
                }) {
                    if m.message_type_id == msg_type::SET_CHUNK_SIZE {
                        if let Some(ProtocolControl::SetChunkSize(size)) =
                            ProtocolControl::from_message(&m).expect("valid set chunk size")
                        {
                            assembler.set_chunk_size(size);
                        }
                        continue;
                    }
                    out.push((m.message_type_id, m.timestamp, m.payload));
                }
                out
            };
            assert_eq!(
                carried, expected,
                "{rel} at chunk size {chunk_size}: every FLV tag crosses as one message"
            );

            let rtmp_media = RtmpDemux::new()
                .unpackage(&wire)
                .unwrap_or_else(|e| panic!("{rel} at chunk size {chunk_size}: RTMP → IR: {e}"));

            assert_eq!(
                track_summary(&rtmp_media),
                track_summary(&ground_truth),
                "{rel} at chunk size {chunk_size}: track/sample shape"
            );
            assert_eq!(
                sample_bytes(&rtmp_media),
                sample_bytes(&ground_truth),
                "{rel} at chunk size {chunk_size}: sample bytes"
            );
        }
    }
}

#[test]
fn the_default_chunk_size_is_the_rtmp_runtime_default() {
    assert_eq!(
        RtmpMux::default().chunk_size,
        usize::try_from(rtmp_runtime::chunk::DEFAULT_CHUNK_SIZE).expect("default chunk size fits"),
    );
}
/// The first message of a chunk stream, decoded incrementally (a Set Chunk
/// Size takes effect for the chunks that follow it within the same buffer).
fn first_message(wire: &[u8]) -> Option<rtmp_runtime::chunk::Message> {
    let mut assembler = ChunkAssembler::new();
    assembler.feed(wire);
    assembler.next_message().expect("chunk stream parses")
}

#[test]
fn a_non_default_chunk_size_is_announced_in_band() {
    let bytes = test_fixtures::read(FIXTURES[0]);
    let media = normalised_media(bytes.as_slice());

    let wire = RtmpMux::with_chunk_size(4096)
        .package(&media)
        .expect("IR → RTMP at 4096");
    let first = first_message(&wire).expect("a non-empty chunk stream");
    assert_eq!(
        first.message_type_id,
        msg_type::SET_CHUNK_SIZE,
        "the stream opens by announcing its chunk size"
    );
    assert_eq!(
        ProtocolControl::from_message(&first).expect("valid protocol control"),
        Some(ProtocolControl::SetChunkSize(4096)),
        "the announced size is the one the muxer chunks at"
    );

    let default_wire = RtmpMux::new().package(&media).expect("IR → RTMP at 128");
    let default_first = first_message(&default_wire).expect("a non-empty chunk stream");
    assert_ne!(
        default_first.message_type_id,
        msg_type::SET_CHUNK_SIZE,
        "at the §5.4.1 default there is nothing to announce"
    );
}

#[test]
fn the_largest_chunk_size_round_trips_through_the_demuxer() {
    let media = normalised_media(test_fixtures::read(FIXTURES[0]).as_slice());
    let wire = RtmpMux::with_chunk_size(rtmp_runtime::chunk::MAX_CHUNK_SIZE as usize)
        .package(&media)
        .expect("MAX_CHUNK_SIZE is a legal announced chunk size");
    assert_eq!(
        ProtocolControl::from_message(&first_message(&wire).expect("a non-empty stream"))
            .expect("valid protocol control"),
        Some(ProtocolControl::SetChunkSize(
            rtmp_runtime::chunk::MAX_CHUNK_SIZE
        )),
        "the announced size is the maximum, not a clamped one"
    );
    let demuxed = RtmpDemux::new()
        .unpackage(&wire)
        .expect("a MAX_CHUNK_SIZE stream demuxes");
    assert_eq!(sample_bytes(&demuxed), sample_bytes(&media));
}

#[test]
fn a_chunk_size_above_the_maximum_is_rejected_not_announced() {
    let media = normalised_media(test_fixtures::read(FIXTURES[0]).as_slice());
    let too_large = rtmp_runtime::chunk::MAX_CHUNK_SIZE as usize + 1;
    let err = RtmpMux::with_chunk_size(too_large)
        .package(&media)
        .expect_err("a chunk size the writer would clamp must be an error");
    assert_eq!(
        err,
        transmux::rtmp::RtmpFlvError::ChunkSizeTooLarge { size: too_large }
    );
}

#[test]
fn a_zero_chunk_size_is_rejected() {
    let media = normalised_media(test_fixtures::read(FIXTURES[0]).as_slice());
    let err = RtmpMux::with_chunk_size(0)
        .package(&media)
        .expect_err("a chunk size of 0 must be an error");
    assert_eq!(
        err,
        transmux::rtmp::RtmpFlvError::ChunkSizeTooLarge { size: 0 }
    );
}

/// The FLV tags [`FlvMux`] wrote for `media`, walked with `rtmp-runtime`'s
/// strict tag walker (no hand-rolled framing here), as `(tag type, body)`.
fn flv_mux_tags(media: &Media) -> Vec<(u8, Vec<u8>)> {
    let flv = FlvMux::new().package(media).expect("IR → FLV");
    let header = <rtmp_runtime::flv::FlvHeader<'_> as Parse>::parse(&flv).expect("FLV header");
    rtmp_runtime::flv::tags(&flv, &header)
        .map(|tag| tag.map(|tag| (tag.tag_type.to_byte(), tag.body.to_vec())))
        .collect::<Result<Vec<_>, _>>()
        .expect("FLV tags walk")
}

/// Bytes of the coded sample [`media_with_video_sample`] carries: enough that
/// all three of the AVC config tag, the data tag and the sample tag stay short
/// of the probe's own size, so one extra byte is one extra message byte.
const MEDIUM_SAMPLE_LEN: usize = 1024;

/// The body length of the longest message for a [`MEDIUM_SAMPLE_LEN`] sample,
/// after checking the message grows byte for byte with the sample: `FlvMux`
/// writes a fixed AVC config tag, a data tag and the one video tag, so only
/// the video tag tracks `sample_len`.
fn longest_message_at_medium_sample() -> usize {
    let small = largest_message_body(&media_with_video_sample(MEDIUM_SAMPLE_LEN));
    let larger = largest_message_body(&media_with_video_sample(MEDIUM_SAMPLE_LEN + 7));
    assert_eq!(
        larger - small,
        7,
        "the longest message grows byte for byte with the sample"
    );
    small
}

/// A [`Media`] carrying the fixture's AVC config and one video sample of
/// `sample_len` bytes.
fn media_with_video_sample(sample_len: usize) -> Media {
    let source = FlvDemux::new()
        .unpackage(test_fixtures::read(FIXTURES[0]).as_slice())
        .expect("fixture FLV → IR");
    let video = source
        .tracks
        .iter()
        .find(|t| matches!(t.config(), transmux::ir::CodecConfig::Avc { .. }))
        .expect("fixture carries a video track");
    let spec = video.spec.clone();
    let sample = transmux::Sample::new(vec![0u8; sample_len], Some(0), Some(0), None, true);
    let track = transmux::Track::new(spec.clone(), vec![sample]);
    Media::new(vec![track], spec.timescale.get())
}

/// The longest message body [`FlvMux`] writes for one video sample of
/// `sample_len` bytes.
fn largest_message_body(media: &Media) -> usize {
    flv_mux_tags(media)
        .iter()
        .map(|(_, body)| body.len())
        .max()
        .expect("the FLV mux writes at least one tag")
}

#[test]
fn a_message_the_demuxer_would_reject_is_refused() {
    let max = rtmp_runtime::chunk::MAX_MESSAGE_LEN as usize;
    let probe = longest_message_at_medium_sample();
    let at_max = media_with_video_sample(MEDIUM_SAMPLE_LEN + (max - probe));
    assert_eq!(
        largest_message_body(&at_max),
        max,
        "a body of exactly the limit"
    );
    let over = media_with_video_sample(MEDIUM_SAMPLE_LEN + (max - probe) + 1);
    assert_eq!(
        largest_message_body(&over),
        max + 1,
        "one byte over the limit"
    );

    let err = RtmpMux::new()
        .package(&over)
        .expect_err("a message over MAX_MESSAGE_LEN must not be framed");
    assert_eq!(
        err,
        transmux::rtmp::RtmpFlvError::MessageTooLarge { len: max + 1, max }
    );
}

/// The messages an RTMP chunk stream carries, every Set Chunk Size applied in
/// turn (`(message type id, body length)`): a chunk stream can carry several
/// messages, so a whole-buffer demux is not how a receiver consumes it.
fn carried_messages(wire: &[u8]) -> Vec<(u8, usize)> {
    let mut assembler = ChunkAssembler::new();
    assembler.feed(wire);
    let mut out = Vec::new();
    while let Some(message) = assembler.next_message().expect("chunk stream parses") {
        if message.message_type_id == msg_type::SET_CHUNK_SIZE {
            if let Some(ProtocolControl::SetChunkSize(size)) =
                ProtocolControl::from_message(&message).expect("valid set chunk size")
            {
                assembler.set_chunk_size(size);
            }
            continue;
        }
        out.push((message.message_type_id, message.payload.len()));
    }
    out
}

#[test]
fn a_message_at_the_maximum_round_trips() {
    let max = rtmp_runtime::chunk::MAX_MESSAGE_LEN as usize;
    let probe = longest_message_at_medium_sample();
    let media = media_with_video_sample(MEDIUM_SAMPLE_LEN + (max - probe));
    let expected = flv_mux_tags(&media);
    let wire = RtmpMux::new()
        .package(&media)
        .expect("a message of exactly MAX_MESSAGE_LEN is framed");
    let video_len = expected
        .iter()
        .map(|(_, body)| body.len())
        .max()
        .expect("at least one tag");
    assert_eq!(video_len, max, "the video tag body is exactly the limit");
    assert_eq!(
        carried_messages(&wire),
        expected
            .iter()
            .map(|(tag_type, body)| {
                let message_type_id = match *tag_type {
                    FLV_TAG_AUDIO => msg_type::AUDIO,
                    FLV_TAG_VIDEO => msg_type::VIDEO,
                    _ => msg_type::DATA_AMF0,
                };
                (message_type_id, body.len())
            })
            .collect::<Vec<_>>(),
        "every FLV tag crosses as one message at exactly its own length"
    );
}

#[test]
fn a_set_chunk_size_with_the_reserved_bit_set_is_rejected() {
    // RTMP 1.0 §5.4.1 requires the reserved top bit to be zero. rtmp-runtime
    // refuses the message (it does not mask the bit), so the demux fails loudly
    // instead of streaming on.
    let mut writer = rtmp_runtime::chunk::ChunkWriter::new();
    let wire = writer
        .write(&rtmp_runtime::chunk::Message {
            chunk_stream_id: 2,
            timestamp: 0,
            message_type_id: msg_type::SET_CHUNK_SIZE,
            message_stream_id: 0,
            payload: RESERVED_BIT_CHUNK_SIZE.to_be_bytes().to_vec(),
        })
        .expect("the chunk writer frames the hostile control message");
    assert!(matches!(
        RtmpDemux::new().unpackage(&wire),
        Err(transmux::rtmp::RtmpFlvError::Rtmp(
            rtmp_runtime::RtmpError::Malformed { .. }
        ))
    ));
}

/// A Set Chunk Size of 4096 with the reserved top bit set.
const RESERVED_BIT_CHUNK_SIZE: u32 = 0x8000_1000;
