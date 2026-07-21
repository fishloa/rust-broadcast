//! Drive the sans-IO LL-HLS **client** engine (`ll_hls_runtime::client`)
//! against a canned Media Playlist — no socket, no real network. The
//! playlist text itself comes from this crate's own origin renderer
//! ([`ll_hls_runtime::server::media_playlist_m3u8`]), so it is guaranteed
//! well-formed LL-HLS syntax (the exact symmetric counterpart
//! `MediaPlaylist::parse` is written against) rather than hand-typed text
//! that could drift from what the parser actually accepts.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example client_stepping -p ll-hls-runtime
//! ```

use ll_hls_runtime::client::{Action, LlHlsClient};
use ll_hls_runtime::server::{DEFAULT_TRACK_ID, MediaStore, media_playlist_m3u8};
use transmux::ll_hls::{PartInfo, SegmentInfo};

const PLAYLIST_URL: &str = "http://origin/live/media.m3u8";

/// Builds a small, valid canned Media Playlist: one closed segment plus an
/// open segment with one part already landed — enough to exercise an init
/// fetch, a part fetch, a preload-hint prefetch, and (since this crate's own
/// renderer defaults `CAN-BLOCK-RELOAD=YES`) a Blocking Playlist Reload for
/// the next request.
fn canned_playlist() -> String {
    let store = MediaStore::new(1.0, 500, 4);
    store.set_init(vec![0xAA; 32]);
    store.add_part(PartInfo {
        bytes: vec![0x01; 16],
        duration: 0.5,
        independent: true,
        segment_seq: 1,
        part_index: 0,
    });
    store.add_segment(SegmentInfo {
        bytes: vec![0x02; 32],
        duration: 1.0,
        segment_seq: 1,
        part_count: 1,
    });
    store.add_part(PartInfo {
        bytes: vec![0x03; 16],
        duration: 0.5,
        independent: true,
        segment_seq: 2,
        part_index: 0,
    });
    media_playlist_m3u8(&store, DEFAULT_TRACK_ID)
}

fn main() {
    let playlist = canned_playlist();
    println!("--- canned media.m3u8 ---\n{playlist}");

    let mut client = LlHlsClient::new(PLAYLIST_URL);

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
