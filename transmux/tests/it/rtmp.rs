//! RTMP transport spoke integration tests (issue #515).
//!
//! Exercises `transmux::rtmp` — the RTMP ⇄ FLV ⇄ IR tie against the real
//! fixture `fixtures/flv/av.flv`. The RTMP wire layer itself (handshake §5.2,
//! chunk headers §5.3.1, chunk reassembly §5.3, protocol control §5.4, AMF0
//! commands §7) lives in the `rtmp-runtime` crate and is covered by its own
//! tests.
//!
//! Gates (each bites):
//! 1. RTMP → IR equals FLV → IR (track/sample counts + per-sample sizes).
//! 2. IR → RTMP wire → IR preserves every sample (no raw passthrough).

use broadcast_common::{Package, Unpackage};
use rtmp_runtime::chunk::{ChunkWriter, Message};
use rtmp_runtime::message::MessageTypeId;
use transmux::FlvDemux;
use transmux::rtmp::{RtmpDemux, RtmpMux};

static FLV: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_fixtures::read("flv/av.flv"));

// ---------------------------------------------------------------------------
// Test 1 — RTMP ⇄ FLV ⇄ IR tie (the real-data bite)
// ---------------------------------------------------------------------------

/// Split `av.flv` into its FLV tags, wrap each A/V tag body as an RTMP message,
/// chunk them at a small chunk size (so video spans multiple chunks), then run
/// `RtmpDemux` → `Media`. Assert the result equals `FlvDemux` on the same FLV.
#[test]
fn rtmp_demux_matches_flv_demux() {
    const CHUNK: usize = 128;

    // Ground truth: FLV → IR directly.
    let mut flv_demux = FlvDemux::new();
    let flv_media = flv_demux.unpackage(FLV.as_slice()).expect("FLV → IR");

    // Split av.flv into FLV tags (reuse the same walk the FLV spoke uses).
    let tags = split_flv(FLV.as_slice());
    assert!(!tags.is_empty(), "av.flv has tags");

    // Wrap each A/V (and script) tag body as an RTMP message on its per-kind
    // csid. Video tags are large enough that at CHUNK=128 they span multiple
    // chunks.
    let mut messages = Vec::new();
    let mut saw_multichunk_video = false;
    for (tag_type, ts, body) in &tags {
        let (csid, mt) = match *tag_type {
            8 => (4u32, MessageTypeId::Audio),
            9 => (5u32, MessageTypeId::Video),
            18 => (6u32, MessageTypeId::DataAmf0),
            _ => continue,
        };
        if mt == MessageTypeId::Video && body.len() > CHUNK {
            saw_multichunk_video = true;
        }
        messages.push(Message {
            chunk_stream_id: csid,
            timestamp: *ts,
            message_type_id: mt,
            message_stream_id: 1,
            payload: body.clone(),
        });
    }
    assert!(
        saw_multichunk_video,
        "at least one video message spans multiple chunks at CHUNK=128"
    );

    // Chunk everything, then RtmpDemux → IR.
    let mut writer = ChunkWriter::new();
    writer.set_chunk_size(u32::try_from(CHUNK).expect("chunk size fits u32"));
    let mut wire = Vec::new();
    for message in &messages {
        wire.extend_from_slice(&writer.write(message).expect("chunk message"));
    }
    let mut rtmp_demux = RtmpDemux::new();
    let rtmp_media = rtmp_demux.unpackage(&wire).expect("RTMP → IR");

    // Track count equal.
    assert_eq!(
        rtmp_media.tracks.len(),
        flv_media.tracks.len(),
        "RTMP and FLV yield the same track count"
    );

    // Per-track: sample count and per-sample payload sizes equal (this proves
    // the chunk reassembly + FLV routing rebuilt the exact A/V bodies — a
    // passthrough could not fake it because it went through RTMP chunking).
    for (r, f) in rtmp_media.tracks.iter().zip(&flv_media.tracks) {
        assert_eq!(
            r.samples.len(),
            f.samples.len(),
            "sample count matches per track"
        );
        for (i, (rs, fs)) in r.samples.iter().zip(&f.samples).enumerate() {
            assert_eq!(rs.data.len(), fs.data.len(), "sample {i} payload size");
            assert_eq!(rs.data, fs.data, "sample {i} payload bytes");
            assert_eq!(rs.flags.is_sync, fs.flags.is_sync, "sample {i} sync flag");
            assert_eq!(
                rs.composition_offset().expect("offset").expect("offset"),
                fs.composition_offset().expect("offset").expect("offset"),
                "sample {i} composition offset"
            );
        }
    }
}

/// IR → `RtmpMux` → wire → `RtmpDemux` → IR must preserve the sample set.
/// A raw-passthrough `RtmpMux` cannot pass: the bytes are re-chunked and the A/V
/// bodies re-framed as RTMP messages, then de-chunked and FLV-routed back.
#[test]
fn rtmp_mux_round_trip_preserves_samples() {
    let mut flv_demux = FlvDemux::new();
    let media = flv_demux.unpackage(FLV.as_slice()).expect("FLV → IR");

    let mut mux = RtmpMux::new();
    let wire = mux.package(&media).expect("IR → RTMP wire");
    assert!(!wire.is_empty(), "muxed RTMP wire is non-empty");

    let mut demux = RtmpDemux::new();
    let back = demux.unpackage(&wire).expect("RTMP wire → IR");

    assert_eq!(
        back.tracks.len(),
        media.tracks.len(),
        "track count preserved"
    );
    for (b, m) in back.tracks.iter().zip(&media.tracks) {
        assert_eq!(b.samples.len(), m.samples.len(), "sample count preserved");
        for (i, (bs, ms)) in b.samples.iter().zip(&m.samples).enumerate() {
            assert_eq!(bs.data, ms.data, "sample {i} bytes preserved");
            assert_eq!(
                bs.flags.is_sync, ms.flags.is_sync,
                "sample {i} sync flag preserved"
            );
        }
    }
}

/// Minimal FLV tag walk for the test (mirrors Adobe FLV v10.1 §E.4.1).
fn split_flv(flv: &[u8]) -> Vec<(u8, u32, Vec<u8>)> {
    let data_offset = u32::from_be_bytes([flv[5], flv[6], flv[7], flv[8]]) as usize;
    let mut off = data_offset.max(9) + 4;
    let mut tags = Vec::new();
    while off + 11 <= flv.len() {
        let tag_type = flv[off];
        let data_size = ((flv[off + 1] as usize) << 16)
            | ((flv[off + 2] as usize) << 8)
            | flv[off + 3] as usize;
        let ts_lo =
            ((flv[off + 4] as u32) << 16) | ((flv[off + 5] as u32) << 8) | flv[off + 6] as u32;
        let ts = ((flv[off + 7] as u32) << 24) | ts_lo;
        let body_start = off + 11;
        let body_end = body_start + data_size;
        if body_end + 4 > flv.len() {
            break;
        }
        tags.push((tag_type, ts, flv[body_start..body_end].to_vec()));
        off = body_end + 4;
    }
    tags
}
