//! Drive the sans-IO LL-HLS **client** engine (`hls_runtime::client`)
//! against a canned Media Playlist — no socket, no real network. The
//! playlist text itself comes from this crate's own origin engine
//! ([`hls_runtime::server::HlsOrigin`]), so it is guaranteed
//! well-formed LL-HLS syntax (the exact symmetric counterpart
//! `MediaPlaylist::parse` is written against) rather than hand-typed text
//! that could drift from what the parser actually accepts.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example client_stepping -p hls-runtime
//! ```

use std::num::NonZeroUsize;
use std::time::Duration;

use broadcast_common::Timestamp;
use hls_runtime::client::{Action, HlsClient};
use hls_runtime::server::{BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsRequest};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::trunk::{PartEntry, SegmentEntry, Trunk, TrunkConfig};
use transmux::SegmentMeta;

const PLAYLIST_URL: &str = "http://origin/live/media.m3u8";

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("example capacity must be non-zero")
}

/// Builds a small, valid canned Media Playlist: one closed segment plus an
/// open segment with one part already landed — enough to exercise an init
/// fetch, a part fetch, a preload-hint prefetch, and (since this crate's own
/// renderer defaults `CAN-BLOCK-RELOAD=YES`) a Blocking Playlist Reload for
/// the next request.
fn canned_playlist() -> String {
    let trunk = Trunk::new(TrunkConfig::new(nz(16), nz(4), nz(8), nz(4), nz(16)));
    let writer = trunk
        .segment_writer()
        .expect("first (and only) segment writer");
    let origin = HlsOrigin::builder(std::sync::Arc::clone(&trunk))
        .target_duration_secs(1.0)
        .window_segments(nz(4))
        .low_latency(500)
        .build()
        .expect("both required fields set");
    origin.set_init(vec![0xAA; 32]);

    writer.publish_part(PartEntry::new(
        vec![0x01; 16],
        1,
        0,
        Duration::from_millis(500),
        true,
    ));
    writer.publish_segment(SegmentEntry::new(
        vec![0x02; 32],
        1,
        Duration::from_secs(1),
        Timestamp::from_nanos(0),
        SegmentMeta {
            discontinuous: false,
        },
    ));
    writer.publish_part(PartEntry::new(
        vec![0x03; 16],
        2,
        0,
        Duration::from_millis(500),
        true,
    ));

    match origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(0)),
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected Ready(Playlist), got {other:?}"),
    }
}

fn main() {
    let playlist = canned_playlist();
    println!("--- canned media.m3u8 ---\n{playlist}");

    let mut client = HlsClient::new(PLAYLIST_URL);

    // The client always seeds a plain (non-blocking) GET first — it hasn't
    // seen a playlist yet, so it doesn't know the origin supports blocking
    // reload.
    match client.poll() {
        Some(Action::FetchPlaylist {
            url,
            blocking,
            skip,
        }) => {
            assert_eq!(url, PLAYLIST_URL);
            assert!(blocking.is_none());
            assert!(!skip);
            println!("action: FetchPlaylist {{ url: {url:?}, blocking: None }}");
        }
        other => panic!("expected the seeded FetchPlaylist, got {other:?}"),
    }

    // Feed the canned playlist in response to that (imagined) GET — no HTTP
    // client is ever involved.
    client
        .on_playlist(playlist.as_bytes())
        .expect("the canned playlist parses");

    // Drain every action the client now wants performed: the closed
    // segment's bytes, the open segment's landed part, the init segment
    // (from `#EXT-X-MAP`), the preload-hinted next part, and finally a
    // Blocking Playlist Reload naming the next Media Sequence Number/part.
    let mut saw_blocking_reload = false;
    while let Some(action) = client.poll() {
        match &action {
            Action::FetchResource { id, url, .. } => {
                println!("action: FetchResource {{ id: {id:?}, url: {url:?} }}");
            }
            Action::FetchPlaylist {
                url,
                blocking: Some(b),
                ..
            } => {
                println!(
                    "action: FetchPlaylist {{ url: {url:?}, blocking: {b:?} }}  <- blocking reload"
                );
                saw_blocking_reload = true;
            }
            Action::FetchPlaylist {
                url,
                blocking: None,
                ..
            } => {
                println!("action: FetchPlaylist {{ url: {url:?}, blocking: None }}");
            }
            Action::WaitMs(ms) => println!("action: WaitMs({ms})"),
            // `Action` is `#[non_exhaustive]` — a future variant is simply
            // not printed by this demo, not a compile break.
            _ => {}
        }
    }
    assert!(
        saw_blocking_reload,
        "this crate's own origin renderer defaults CAN-BLOCK-RELOAD=YES, so the \
         next reload the client schedules must be a blocking one"
    );
}
