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

        let parts: Vec<&str> = s.lines().collect();
        let id = parts.first().copied().unwrap_or("");
        if let Ok(mut range) = broadcast_hls::DateRange::new(id) {
            if let Some(class) = parts.get(1) {
                let _ = range.set_class(Some(*class));
            }
            if let Some(value) = parts.get(2)
                && let Ok(value) = broadcast_hls::AttrValue::quoted(*value)
                && let Ok(name) = broadcast_hls::ClientAttrName::new("X-FUZZ")
            {
                let _ = range.set_client_attr(name, value);
            }
            if let Ok(cue) = broadcast_hls::Cue::new(vec![broadcast_hls::CueTrigger::Once]) {
                range.set_cue(Some(cue));
            }
            if let Some(hex) = parts.get(3) {
                let _ = range.set_scte35(broadcast_hls::Scte35Kind::Out, *hex);
            }
            if let Ok(rendered) = range.to_tag_line() {
                let reparsed = broadcast_hls::DateRange::parse_tag_line(&rendered)
                    .expect("a setter-built DATERANGE must parse");
                assert_eq!(reparsed, range, "parse(render(x)) must equal x");
                assert_eq!(
                    reparsed.to_tag_line().expect("re-render"),
                    rendered,
                    "render(parse(render(x))) must equal render(x)"
                );
            }
        }

        if let Ok(name) = broadcast_hls::CustomTagName::new("EXT-X-FUZZ") {
            let mut tag = broadcast_hls::CustomTag::new(name);
            if let Some(value) = parts.first()
                && let Ok(value) = broadcast_hls::AttrValue::quoted(*value)
            {
                let _ = tag.set_attr("A", value);
            }
            if let Some(value) = parts.get(1)
                && let Ok(value) = broadcast_hls::AttrValue::bare(*value)
            {
                let _ = tag.set_attr("B", value);
            }
            if let Some(value) = parts.get(2)
                && let Ok(value) = broadcast_hls::AttrValue::quoted(*value)
            {
                let _ = tag.set_attr("A", value);
            }
            if let Ok(rendered) = tag.to_tag_line() {
                let reparsed = broadcast_hls::CustomTag::parse_tag_line(&rendered)
                    .expect("a setter-built custom tag must parse");
                assert_eq!(reparsed, tag, "parse(render(x)) must equal x");
                assert_eq!(
                    reparsed.to_tag_line().expect("re-render"),
                    rendered,
                    "render(parse(render(x))) must equal render(x)"
                );
            }
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
