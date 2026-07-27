//! Drive the sans-IO LL-HLS **origin** engine (`ll_hls_runtime::server`)
//! with zero IO: publish synthetic init/segment/part bytes into a
//! [`Trunk`], then render playlists and resolve resources through
//! [`LlHlsOrigin`]'s [`ServedEgress`] impl exactly as an HTTP adapter would —
//! no socket, no clock, no async runtime.
//!
//! A real pipeline (a segmenter feeding a `TrunkWriter`) would publish real
//! encoded media; here the bytes are synthetic placeholders, since this
//! example is about the origin engine's *decision logic* (blocking-reload/
//! part availability, playlist rendering), not encoding.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example origin_playlist -p ll-hls-runtime
//! ```

use std::num::NonZeroUsize;
use std::time::Duration;

use broadcast_common::Timestamp;
use ll_hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, LlHlsBody, LlHlsOrigin, LlHlsRequest, master_playlist_m3u8,
};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::trunk::{PartEntry, SegmentEntry, Trunk, TrunkConfig};
use transmux::SegmentMeta;

/// Target full-segment duration, in seconds.
const TARGET_DURATION_SECS: f64 = 1.0;
/// LL-HLS part target, in milliseconds.
const PART_TARGET_MS: u32 = 500;
/// Rolling window depth: full segments this origin advertises in a
/// rendered playlist.
const WINDOW_SEGMENTS: usize = 4;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("example capacity must be non-zero")
}

/// Every call in this example is a plain, non-blocking `resolve` — no
/// request here ever needs to wait, so `now`/`await_policy` are nominal
/// zero values throughout.
fn resolve(origin: &LlHlsOrigin, request: LlHlsRequest) -> EgressResponse<LlHlsBody> {
    origin.resolve(
        request,
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(0)),
    )
}

fn main() {
    let trunk = Trunk::new(TrunkConfig::new(nz(16), nz(4), nz(8), nz(4), nz(16)));
    let writer = trunk.writer().expect("first (and only) writer");
    let origin = LlHlsOrigin::new(
        std::sync::Arc::clone(&trunk),
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
        nz(WINDOW_SEGMENTS),
    );
    origin.set_init(vec![0xAA; 32]);

    // Segment 1 closes with two parts.
    writer.publish_part(PartEntry::new(
        vec![0x01; 16],
        1,
        0,
        Duration::from_millis(500),
        true,
    ));
    writer.publish_part(PartEntry::new(
        vec![0x02; 16],
        1,
        1,
        Duration::from_millis(500),
        false,
    ));
    writer.publish_segment(SegmentEntry::new(
        vec![0x03; 32],
        1,
        Duration::from_secs(1),
        Timestamp::from_nanos(0),
        SegmentMeta {
            discontinuous: false,
        },
    ));

    // Segment 2 is still open, with only its first part landed so far.
    writer.publish_part(PartEntry::new(
        vec![0x04; 16],
        2,
        0,
        Duration::from_millis(500),
        true,
    ));

    println!("--- master.m3u8 ---");
    println!("{}", master_playlist_m3u8("media.m3u8"));

    println!("--- media.m3u8 ---");
    match resolve(
        &origin,
        LlHlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
    ) {
        EgressResponse::Ready {
            body: LlHlsBody::Playlist(m),
            ..
        } => println!("{m}"),
        other => panic!("expected Ready(Playlist), got {other:?}"),
    }

    // A plain (non-blocking) request is Ready immediately.
    let outcome = resolve(
        &origin,
        LlHlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
    );
    assert!(matches!(
        outcome,
        EgressResponse::Ready {
            body: LlHlsBody::Playlist(_),
            ..
        }
    ));
    println!("resolve(Playlist, no query)     -> Ready");

    // A blocking-reload request for a segment that hasn't closed yet: with
    // `await_policy`'s deadline already at `now`, this immediately reports
    // the awaited condition has run out of patience rather than serving a
    // fabricated Ready.
    let outcome = resolve(
        &origin,
        LlHlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery {
                hls_msn: Some(5),
                hls_part: None,
            },
        },
    );
    assert_eq!(outcome, EgressResponse::NotFound);
    println!("resolve(Playlist, _HLS_msn=5)   -> NotFound (Await's patience already expired)");

    // A `_HLS_msn` unreasonably far beyond the live edge is rejected outright
    // (RFC 8216bis §6.2.5.2 abuse prevention) rather than ever Await-ing.
    let outcome = resolve(
        &origin,
        LlHlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery {
                hls_msn: Some(999),
                hls_part: None,
            },
        },
    );
    assert!(matches!(outcome, EgressResponse::BadRequest { .. }));
    println!("resolve(Playlist, _HLS_msn=999) -> BadRequest (abuse bound)");

    // `Resource`: the init segment and the closed segment are Ready...
    match resolve(
        &origin,
        LlHlsRequest::Resource {
            name: "init-1.mp4".to_string(),
        },
    ) {
        EgressResponse::Ready { .. } => println!("resolve(Resource, init-1.mp4)     -> Ready"),
        other => panic!("expected Ready, got {other:?}"),
    }
    match resolve(
        &origin,
        LlHlsRequest::Resource {
            name: "seg-1-1.m4s".to_string(),
        },
    ) {
        EgressResponse::Ready { .. } => println!("resolve(Resource, seg-1-1.m4s)    -> Ready"),
        other => panic!("expected Ready, got {other:?}"),
    }
    // ...a live part of the still-open segment is Ready too...
    match resolve(
        &origin,
        LlHlsRequest::Resource {
            name: "part-1-2.0.m4s".to_string(),
        },
    ) {
        EgressResponse::Ready { .. } => println!("resolve(Resource, part-1-2.0.m4s) -> Ready"),
        other => panic!("expected Ready, got {other:?}"),
    }
    // ...a preload-hinted part not yet produced reports NotFound once this
    // call's `await_policy` has already expired (a real HTTP adapter would
    // instead give it a real deadline and block on `Trunk::listen()`)...
    match resolve(
        &origin,
        LlHlsRequest::Resource {
            name: "part-1-2.1.m4s".to_string(),
        },
    ) {
        EgressResponse::NotFound => {
            println!(
                "resolve(Resource, part-1-2.1.m4s) -> NotFound (Await's patience already expired)"
            )
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
    // ...and an unrecognised filename is a plain 404.
    match resolve(
        &origin,
        LlHlsRequest::Resource {
            name: "nope.txt".to_string(),
        },
    ) {
        EgressResponse::NotFound => println!("resolve(Resource, nope.txt)       -> NotFound"),
        other => panic!("expected NotFound, got {other:?}"),
    }
}
