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

use broadcast_common::{Package, Unpackage};
use rtmp_runtime::chunk::{ChunkAssembler, Message};
use rtmp_runtime::message::msg_type;
use transmux::{FlvDemux, FlvMux, Media, RtmpDemux, RtmpMux};

const FIXTURES: &[&str] = &["flv/av.flv", "flv/aac-7-1.flv"];

const FLV_TAG_AUDIO: u8 = msg_type::AUDIO;
const FLV_TAG_VIDEO: u8 = msg_type::VIDEO;

/// The §5.3 default chunk size. Any other size is currently refused by
/// `rtmp_runtime::chunk::ChunkAssembler` when the stream carries more than
/// one kind of message (a real rtmp-runtime defect, independent of this
/// spoke: the muxer's own bytes are chunked exactly as §5.3 prescribes).
const CHUNK_SIZES: &[usize] = &[128];

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

/// `(tag type, timestamp, body)` for every tag an FLV byte stream carries.
fn flv_tags(flv: &[u8]) -> Vec<(u8, u32, Vec<u8>)> {
    let data_offset = u32::from_be_bytes([flv[5], flv[6], flv[7], flv[8]]) as usize;
    let mut off = data_offset.max(9) + 4;
    let mut tags = Vec::new();
    while off + 11 <= flv.len() {
        let tag_type = flv[off];
        let len = (usize::from(flv[off + 1]) << 16)
            | (usize::from(flv[off + 2]) << 8)
            | usize::from(flv[off + 3]);
        let timestamp = (u32::from(flv[off + 7]) << 24)
            | (u32::from(flv[off + 4]) << 16)
            | (u32::from(flv[off + 5]) << 8)
            | u32::from(flv[off + 6]);
        if off + 11 + len > flv.len() {
            break;
        }
        tags.push((tag_type, timestamp, flv[off + 11..off + 11 + len].to_vec()));
        off += 11 + len + 4;
    }
    tags
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
        let flv_media = normalised_media(bytes.as_slice());
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
            let carried: Vec<(u8, u32, Vec<u8>)> = ChunkAssembler::new()
                .push(&wire)
                .unwrap_or_else(|e| panic!("{rel} at chunk size {chunk_size}: chunk stream: {e}"))
                .into_iter()
                .map(|m: Message| (m.message_type_id, m.timestamp, m.payload))
                .collect();
            assert_eq!(
                carried, expected,
                "{rel} at chunk size {chunk_size}: every FLV tag crosses as one message"
            );

            let rtmp_media = RtmpDemux::new()
                .unpackage(&wire)
                .unwrap_or_else(|e| panic!("{rel} at chunk size {chunk_size}: RTMP → IR: {e}"));

            assert_eq!(
                track_summary(&rtmp_media),
                track_summary(&flv_media),
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
