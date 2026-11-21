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
        ) && let Ok(rendered) = playlist.to_m3u8()
        {
            let reparsed = broadcast_hls::MediaPlaylist::parse_with_options(
                &rendered,
                &broadcast_hls::limits::Limits::default(),
                &options,
            )
            .expect("a rendered playlist must parse");
            assert_eq!(
                reparsed.to_m3u8().expect("re-render"),
                rendered,
                "render(parse(render(x))) must equal render(x)"
            );
        }

        for line in s.lines() {
            if let Ok(range) = broadcast_hls::DateRange::parse_tag_line(line)
                && let Ok(rendered) = range.to_tag_line()
            {
                let reparsed = broadcast_hls::DateRange::parse_tag_line(&rendered)
                    .expect("a rendered DATERANGE must parse");
                assert_eq!(reparsed, range, "parse(render(x)) must equal x");
                assert_eq!(
                    reparsed.to_tag_line().expect("re-render"),
                    rendered,
                    "render(parse(render(x))) must equal render(x)"
                );
            }
            if let Ok(tag) = broadcast_hls::CustomTag::parse_tag_line(line)
                && let Ok(rendered) = tag.to_tag_line()
            {
                let reparsed = broadcast_hls::CustomTag::parse_tag_line(&rendered)
                    .expect("a rendered custom tag must parse");
                assert_eq!(reparsed, tag, "parse(render(x)) must equal x");
                assert_eq!(
                    reparsed.to_tag_line().expect("re-render"),
                    rendered,
                    "render(parse(render(x))) must equal render(x)"
                );
            }
        }
    }
});
