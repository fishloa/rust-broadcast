//! Drive the sans-IO LL-HLS **origin** engine (`ll_hls_runtime::server`)
//! with zero IO: push synthetic init/segment/part bytes into a
//! [`MediaStore`], then render playlists and resolve resources exactly as
//! multimux's real HTTP adapter would — no socket, no clock, no async
//! runtime.
//!
//! A real pipeline (`multimux::pipeline::run_pipeline`, backed by
//! `transmux::ll_hls::LlHlsSegmenter`) would feed the store from encoded
//! media; here the bytes are synthetic placeholders, since this example is
//! about the origin engine's *decision logic* (blocking-reload/part
//! availability, playlist rendering), not encoding.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example origin_playlist -p ll-hls-runtime
//! ```

use ll_hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, MediaStore, PlaylistOutcome, ResourceOutcome,
    master_playlist_m3u8, media_playlist_m3u8,
};
use transmux::ll_hls::{PartInfo, SegmentInfo};

/// Target full-segment duration, in seconds.
const TARGET_DURATION_SECS: f64 = 1.0;
/// LL-HLS part target, in milliseconds.
const PART_TARGET_MS: u32 = 500;
/// Rolling window depth: full segments retained in RAM.
const WINDOW_SEGMENTS: usize = 4;

fn main() {
    let store = MediaStore::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS);
    store.set_init(vec![0xAA; 32]);

    // Segment 1 closes with two parts.
    store.add_part(PartInfo {
        bytes: vec![0x01; 16],
        duration: 0.5,
        independent: true,
        segment_seq: 1,
        part_index: 0,
    });
    store.add_part(PartInfo {
        bytes: vec![0x02; 16],
        duration: 0.5,
        independent: false,
        segment_seq: 1,
        part_index: 1,
    });
    store.add_segment(SegmentInfo {
        bytes: vec![0x03; 32],
        duration: 1.0,
        segment_seq: 1,
        part_count: 2,
    });

    // Segment 2 is still open, with only its first part landed so far.
    store.add_part(PartInfo {
        bytes: vec![0x04; 16],
        duration: 0.5,
        independent: true,
        segment_seq: 2,
        part_index: 0,
    });

    println!("--- master.m3u8 ---");
    println!("{}", master_playlist_m3u8("media.m3u8"));

    println!("--- media.m3u8 ---");
    println!("{}", media_playlist_m3u8(&store, DEFAULT_TRACK_ID));

    // `resolve_playlist`: a plain (non-blocking) request is Ready immediately.
    let outcome = store.resolve_playlist(DEFAULT_TRACK_ID, BlockingQuery::default());
    assert!(matches!(outcome, PlaylistOutcome::Ready(_)));
    println!("resolve_playlist(no query)     -> Ready");

    // A blocking-reload request for a segment that hasn't closed yet blocks.
    let outcome = store.resolve_playlist(
        DEFAULT_TRACK_ID,
        BlockingQuery {
            hls_msn: Some(5),
            hls_part: None,
        },
    );
    assert_eq!(outcome, PlaylistOutcome::WouldBlock);
    println!("resolve_playlist(_HLS_msn=5)   -> WouldBlock");

    // A `_HLS_msn` unreasonably far beyond the live edge is rejected outright
    // (RFC 8216bis §6.2.5.2 abuse prevention) rather than blocking forever.
    let outcome = store.resolve_playlist(
        DEFAULT_TRACK_ID,
        BlockingQuery {
            hls_msn: Some(999),
            hls_part: None,
        },
    );
    assert_eq!(outcome, PlaylistOutcome::BadRequest);
    println!("resolve_playlist(_HLS_msn=999) -> BadRequest (abuse bound)");

    // `resolve_resource`: the init segment and the closed segment are Ready...
    match store.resolve_resource("init-1.mp4") {
        ResourceOutcome::Ready { .. } => println!("resolve_resource(init-1.mp4)     -> Ready"),
        other => panic!("expected Ready, got {other:?}"),
    }
    match store.resolve_resource("seg-1-1.m4s") {
        ResourceOutcome::Ready { .. } => println!("resolve_resource(seg-1-1.m4s)    -> Ready"),
        other => panic!("expected Ready, got {other:?}"),
    }
    // ...a live part of the still-open segment is Ready too...
    match store.resolve_resource("part-1-2.0.m4s") {
        ResourceOutcome::Ready { .. } => println!("resolve_resource(part-1-2.0.m4s) -> Ready"),
        other => panic!("expected Ready, got {other:?}"),
    }
    // ...a preload-hinted part not yet produced would block the caller...
    match store.resolve_resource("part-1-2.1.m4s") {
        ResourceOutcome::WouldBlock => println!("resolve_resource(part-1-2.1.m4s) -> WouldBlock"),
        other => panic!("expected WouldBlock, got {other:?}"),
    }
    // ...and an unrecognised filename is a plain 404.
    match store.resolve_resource("nope.txt") {
        ResourceOutcome::NotFound => println!("resolve_resource(nope.txt)       -> NotFound"),
        other => panic!("expected NotFound, got {other:?}"),
    }
}
