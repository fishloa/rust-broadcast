//! RFC 8216bis §8 version-compatibility triggers, one field at a time.
//!
//! `MasterPlaylist::computed_version` joins its row-8 (variable
//! substitution) triggers with `||` in a single condition. A fixture that
//! trips several at once cannot tell whether each one is still wired in, so
//! every trigger below is exercised ALONE: a playlist whose only `{$var}` is
//! in that one field must still declare version 8 (audit 21 finding 42,
//! issue #2937).

const VERSION_VARIABLE_SUBSTITUTION: u8 = 8;

fn master(media: &str, variant_uri: &str) -> String {
    format!(
        "#EXTM3U\n\
{media}\
#EXT-X-STREAM-INF:BANDWIDTH=1000\n\
{variant_uri}\n"
    )
}

fn media_line(field: &str) -> String {
    format!("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"n\",{field}\n")
}

#[test]
fn every_row_8_trigger_alone_declares_version_8_on_a_master_playlist() {
    let cases: [(&str, String, &str); 12] = [
        ("media.uri", media_line("URI=\"a{$v}.m3u8\""), "v.m3u8"),
        (
            "media.group_id",
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a{$v}\",NAME=\"n\"\n".to_string(),
            "v.m3u8",
        ),
        (
            "media.name",
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"n{$v}\"\n".to_string(),
            "v.m3u8",
        ),
        (
            "media.language",
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"n\",LANGUAGE=\"en{$v}\"\n".to_string(),
            "v.m3u8",
        ),
        (
            "media.assoc_language",
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"n\",ASSOC-LANGUAGE=\"en{$v}\"\n"
                .to_string(),
            "v.m3u8",
        ),
        (
            "media.stable_rendition_id",
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"n\",STABLE-RENDITION-ID=\"s{$v}\"\n"
                .to_string(),
            "v.m3u8",
        ),
        (
            "media.channels",
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"n\",CHANNELS=\"2{$v}\"\n".to_string(),
            "v.m3u8",
        ),
        (
            "media.characteristics",
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"n\",CHARACTERISTICS=\"public.a{$v}\"\n"
                .to_string(),
            "v.m3u8",
        ),
        ("variant.uri", String::new(), "v{$v}.m3u8"),
        (
            "iframe.uri",
            "#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=1,URI=\"i{$v}.m3u8\"\n".to_string(),
            "v.m3u8",
        ),
        (
            "extra_tag",
            "#EXT-X-CUSTOM:flag{$v}\n".to_string(),
            "v.m3u8",
        ),
        (
            "define.value",
            "#EXT-X-DEFINE:NAME=\"x\",VALUE=\"{$v}\"\n".to_string(),
            "v.m3u8",
        ),
    ];

    for (trigger, media, variant_uri) in cases {
        let text = master(&media, variant_uri);
        let pl = broadcast_hls::MasterPlaylist::parse(&text)
            .unwrap_or_else(|e| panic!("{trigger}: {text:?} did not parse: {e:?}"));
        assert_eq!(
            pl.computed_version(),
            Some(VERSION_VARIABLE_SUBSTITUTION),
            "{trigger} alone must require version 8"
        );
    }

    let plain = master("", "v.m3u8");
    let pl = broadcast_hls::MasterPlaylist::parse(&plain).unwrap();
    assert!(
        pl.computed_version().unwrap_or(0) < VERSION_VARIABLE_SUBSTITUTION,
        "a playlist with no substitution must not declare version 8, got {:?}",
        pl.computed_version()
    );
}

#[test]
fn row_8_trigger_via_a_define_value_alone_on_a_media_playlist() {
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
#EXT-X-DEFINE:NAME=\"x\",VALUE=\"{$v}\"\n\
#EXTINF:4,\nseg0.ts\n";
    let pl = broadcast_hls::MediaPlaylist::parse(text).unwrap();
    assert_eq!(pl.computed_version(), Some(VERSION_VARIABLE_SUBSTITUTION));

    let plain = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nseg0.ts\n";
    let pl = broadcast_hls::MediaPlaylist::parse(plain).unwrap();
    assert!(pl.computed_version().unwrap_or(0) < VERSION_VARIABLE_SUBSTITUTION);
}

#[test]
fn keyformat_triggers_row_2_5_version_alone() {
    let base = |tag: &str| {
        format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n{tag}\n#EXTINF:4,\nseg0.ts\n#EXT-X-ENDLIST\n")
    };
    for tag in [
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"",
        "#EXT-X-KEY:METHOD=AES-128,URI=\"k\",KEYFORMAT=\"identity\"",
        "#EXT-X-KEY:METHOD=AES-128,URI=\"k\",KEYFORMATVERSIONS=\"1\"",
    ] {
        let pl = broadcast_hls::MediaPlaylist::parse(&base(tag)).unwrap();
        assert_eq!(
            pl.computed_version(),
            Some(5),
            "{tag} alone must require version 5"
        );
    }
    let plain = broadcast_hls::MediaPlaylist::parse(&base("#EXT-X-CUSTOM:1")).unwrap();
    assert!(plain.computed_version().unwrap_or(0) < 5);
}

#[test]
fn req_attribute_on_a_map_or_a_part_or_the_open_segment_alone_declares_version_12() {
    let base = |lines: &str| format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n{lines}");

    let map_only = base(
        "#EXT-X-MAP:URI=\"init.mp4\",REQ-EXTRA=\"x\"\n\
#EXTINF:4,\nseg0.ts\n#EXT-X-ENDLIST\n",
    );
    assert_eq!(
        broadcast_hls::MediaPlaylist::parse(&map_only)
            .unwrap()
            .computed_version(),
        Some(12),
        "REQ- on EXT-X-MAP alone"
    );

    let part_only = base(
        "#EXTINF:4,\nseg0.ts\n\
#EXT-X-PART:DURATION=1.0,URI=\"p0.m4s\",REQ-EXTRA=\"x\"\n\
#EXT-X-PART:DURATION=1.0,URI=\"p1.m4s\"\n\
#EXT-X-ENDLIST\n",
    );
    assert_eq!(
        broadcast_hls::MediaPlaylist::parse(&part_only)
            .unwrap()
            .computed_version(),
        Some(12),
        "REQ- on a closed segment's EXT-X-PART alone"
    );

    let open_map_only = base(
        "#EXTINF:4,\nseg0.ts\n\
#EXT-X-MAP:URI=\"init.mp4\",REQ-EXTRA=\"x\"\n\
#EXT-X-PART:DURATION=1.0,URI=\"p0.m4s\"\n",
    );
    assert_eq!(
        broadcast_hls::MediaPlaylist::parse(&open_map_only)
            .unwrap()
            .computed_version(),
        Some(12),
        "REQ- on the open segment's EXT-X-MAP alone"
    );

    let open_part_only = base(
        "#EXTINF:4,\nseg0.ts\n\
#EXT-X-PART:DURATION=1.0,URI=\"p0.m4s\",REQ-EXTRA=\"x\"\n",
    );
    assert_eq!(
        broadcast_hls::MediaPlaylist::parse(&open_part_only)
            .unwrap()
            .computed_version(),
        Some(12),
        "REQ- on the open segment's EXT-X-PART alone"
    );
}
