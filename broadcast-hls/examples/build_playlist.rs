//! Build a VOD Media Playlist from typed fields and render it to `#EXTM3U`
//! text — RFC 8216 §4.3.
//!
//! Run with `cargo run -p broadcast-hls --example build_playlist`.

use broadcast_hls::{MediaPlaylist, MediaSegment};

fn main() {
    let playlist = MediaPlaylist {
        version: 3,
        target_duration: 10,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![
            MediaSegment {
                uri: "seg0.m4s".into(),
                duration: 9.009,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg1.m4s".into(),
                duration: 9.009,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg2.m4s".into(),
                duration: 3.003,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
        ],
        endlist: true,
        ..Default::default()
    };

    print!("{}", playlist.to_m3u8());
}
