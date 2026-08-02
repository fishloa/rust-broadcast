#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz `broadcast-hls`'s RFC 8216bis playlist parsers — the inverse of
// `MediaPlaylist::to_m3u8`/`MasterPlaylist::to_m3u8` — on arbitrary UTF-8
// text. Must not panic on any input, however malformed.
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = broadcast_hls::MediaPlaylist::parse(s);
        let _ = broadcast_hls::MasterPlaylist::parse(s);
    }
});
