//! Parse an `#EXTM3U` Media Playlist back into structured fields — the
//! symmetric inverse of [`broadcast_hls::MediaPlaylist::to_m3u8`] (RFC 8216
//! §4.3 / RFC 8216bis).
//!
//! Run with `cargo run -p broadcast-hls --example parse_playlist`.

use broadcast_hls::MediaPlaylist;

const PLAYLIST: &str = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:10\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXTINF:9.009,\n\
seg0.m4s\n\
#EXTINF:9.009,\n\
seg1.m4s\n\
#EXTINF:3.003,\n\
seg2.m4s\n\
#EXT-X-ENDLIST\n";

fn main() {
    let playlist = MediaPlaylist::parse(PLAYLIST).expect("valid playlist");

    println!("version: {}", playlist.version);
    println!("target_duration: {}", playlist.target_duration);
    println!("segments: {}", playlist.segments.len());
    for seg in &playlist.segments {
        println!("  {} ({:.3}s)", seg.uri, seg.duration);
    }
    assert!(playlist.endlist);

    // Round-trip: re-rendering a parsed playlist reproduces the same text.
    assert_eq!(playlist.to_m3u8(), PLAYLIST);
}
