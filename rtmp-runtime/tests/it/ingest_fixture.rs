//! Real-capture ingest fixture (#738 Task 8): replay a genuine `ffmpeg` RTMP
//! publish through [`ServerSession::handle_data`] and confirm the emitted FLV
//! decodes through [`transmux::FlvDemux`] to a real H.264+AAC `Media`.
//!
//! Fixture: `tests/fixtures/obs-publish.bin` — every raw inbound byte a real
//! `ffmpeg -f flv` publisher sent to the sans-IO [`ServerSession`], captured
//! by `examples/capture_publish.rs`. See `tests/fixtures/PROVENANCE.md` for
//! the exact command and capture details.
//!
//! This test is entirely offline: no socket, no tokio — it only replays the
//! committed bytes through the sans-IO session plus `transmux::FlvDemux`, so
//! it runs under `--all-features` (and `--no-default-features`) without the
//! `tokio` feature.

use broadcast_common::Unpackage;
use rtmp_runtime::server::{ServerEvent, ServerSession};
use transmux::{CodecConfig, FlvDemux};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/obs-publish.bin"
);

fn load_fixture() -> Vec<u8> {
    std::fs::read(FIXTURE).expect("read tests/fixtures/obs-publish.bin")
}

/// Feed `input` to a fresh [`ServerSession`] in `chunk_len`-sized pieces
/// (the whole buffer in one call if `chunk_len == 0`), returning every event
/// produced across all calls in order.
fn replay(input: &[u8], chunk_len: usize) -> Vec<ServerEvent> {
    let mut session = ServerSession::with_defaults();
    let mut events = Vec::new();
    if chunk_len == 0 {
        let (_out, evs) = session.handle_data(input).expect("replay must not error");
        events.extend(evs);
    } else {
        for chunk in input.chunks(chunk_len) {
            let (_out, evs) = session.handle_data(chunk).expect("replay must not error");
            events.extend(evs);
        }
    }
    events
}

fn assert_reaches_publishing(events: &[ServerEvent]) {
    assert!(
        events.iter().any(|e| matches!(
            e,
            ServerEvent::Connected { app } if app == "live"
        )),
        "must emit Connected{{app: \"live\"}}; got {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            ServerEvent::Publish { stream_key, .. } if stream_key == "testkey"
        )),
        "must emit Publish{{stream_key: \"testkey\", ..}}; got events: {events:?}"
    );
    let media_count = events
        .iter()
        .filter(|e| matches!(e, ServerEvent::Media { .. }))
        .count();
    assert!(
        media_count >= 1,
        "must emit at least one Media event, got {media_count}"
    );
}

fn concat_flv(events: &[ServerEvent]) -> Vec<u8> {
    let mut flv = Vec::new();
    for e in events {
        if let ServerEvent::Media { flv: tag } = e {
            flv.extend_from_slice(tag);
        }
    }
    flv
}

/// The real capture, replayed in one `handle_data` call, reaches Publishing
/// and emits Media events.
#[test]
fn real_capture_single_call_reaches_publishing() {
    let bytes = load_fixture();
    let events = replay(&bytes, 0);
    assert_reaches_publishing(&events);
}

/// The same real capture, replayed in small chunks (exercising partial
/// handshake/chunk reassembly across many `handle_data` calls), reaches the
/// same milestones.
#[test]
fn real_capture_chunked_reassembly_reaches_publishing() {
    let bytes = load_fixture();
    // A deliberately awkward chunk size: neither a handshake-packet boundary
    // (1537) nor typical chunk-header/message-size boundaries, so partial
    // reads land mid-handshake and mid-chunk repeatedly.
    let events = replay(&bytes, 97);
    assert_reaches_publishing(&events);
}

/// The emitted FLV (concatenated `Media.flv` in arrival order) decodes
/// through `transmux::FlvDemux` to a real `Media` with an H.264 video track
/// and an AAC audio track, each carrying a sane sample count. This is the
/// end-to-end bite: if `ServerSession` dropped the FLV file header, or
/// mis-tagged audio vs video, `FlvDemux` would fail or mis-decode.
#[test]
fn emitted_flv_decodes_through_flv_demux_to_h264_aac_media() {
    let bytes = load_fixture();
    let events = replay(&bytes, 0);
    assert_reaches_publishing(&events);

    let flv_bytes = concat_flv(&events);
    assert!(
        flv_bytes.starts_with(b"FLV"),
        "concatenated Media.flv must start with the FLV file header"
    );

    let mut demux = FlvDemux::new();
    let media = demux
        .unpackage(&flv_bytes)
        .expect("transmux::FlvDemux must decode the ServerSession-emitted FLV");

    assert_eq!(
        media.tracks.len(),
        2,
        "must enumerate exactly 2 tracks (AVC video + AAC audio): got {:?}",
        media.tracks.iter().map(|t| t.config()).collect::<Vec<_>>()
    );

    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("an AVC video track");
    match video.config() {
        CodecConfig::Avc { width, height, .. } => {
            assert_eq!(
                (*width, *height),
                (320, 240),
                "AVC dims decoded from the SPS inside the avcC must match the source fixture"
            );
        }
        other => panic!("expected AVC, got {other:?}"),
    }
    assert!(
        !video.samples.is_empty(),
        "AVC track must have a non-zero sample count"
    );

    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
        .expect("an AAC audio track");
    match audio.config() {
        CodecConfig::Aac { sample_rate, .. } => {
            assert!(
                *sample_rate > 0,
                "AAC sample_rate must be decoded from the ASC (non-zero)"
            );
        }
        other => panic!("expected AAC, got {other:?}"),
    }
    assert!(
        !audio.samples.is_empty(),
        "AAC track must have a non-zero sample count"
    );
}
