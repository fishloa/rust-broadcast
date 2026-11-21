#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz `broadcast-hls`'s RFC 8216bis playlist parsers — the inverse of
// `MediaPlaylist::to_m3u8`/`MasterPlaylist::to_m3u8` — on arbitrary UTF-8
// text. Must not panic on any input, however malformed.
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = broadcast_hls::MediaPlaylist::parse(s);
        let _ = broadcast_hls::MasterPlaylist::parse(s);

        let name = match broadcast_hls::CustomTagName::new("EXT-X-RUSTBROADCAST-TIMING") {
            Ok(name) => name,
            Err(_) => return,
        };
        let options = broadcast_hls::ParseOptions::default().with_segment_scoped_tag(name);
        if let Ok(playlist) = broadcast_hls::MediaPlaylist::parse_with_options(
            s,
            &broadcast_hls::limits::Limits::default(),
            &options,
        ) {
            let _ = playlist.to_m3u8();
        }

        for line in s.lines() {
            if let Ok(range) = broadcast_hls::DateRange::parse_tag_line(line)
                && let Ok(rendered) = range.to_tag_line()
            {
                assert!(broadcast_hls::DateRange::parse_tag_line(&rendered).is_ok());
            }
            if let Ok(tag) = broadcast_hls::CustomTag::parse_tag_line(line)
                && let Ok(rendered) = tag.to_tag_line()
            {
                assert!(broadcast_hls::CustomTag::parse_tag_line(&rendered).is_ok());
            }
        }
    }
});
