use broadcast_hls::{FrameRate, MasterPlaylist, MediaPlaylist};

fn master(media: &str) -> String {
    format!("#EXTM3U\n{media}\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n")
}

#[test]
fn absent_autoselect_is_no_and_does_not_render_as_yes() {
    let text =
        master("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"en\",DEFAULT=YES,URI=\"a.m3u8\"");
    let p = MasterPlaylist::parse(&text).unwrap();
    assert!(p.media[0].default);
    assert!(!p.media[0].autoselect);
    let out = p.to_m3u8().unwrap();
    assert!(!out.contains("AUTOSELECT=YES"), "{out}");
    let again = MasterPlaylist::parse(&out).unwrap();
    assert!(!again.media[0].autoselect);
}

#[test]
fn absent_autoselect_without_default_is_no() {
    let text = master("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"en\",URI=\"a.m3u8\"");
    let p = MasterPlaylist::parse(&text).unwrap();
    assert!(!p.media[0].autoselect);
}

#[test]
fn frame_rate_accepts_any_number_of_fraction_digits() {
    let long = "29.9700299700299700299700299700299700299700299700";
    assert_eq!(
        "29.97003".parse::<FrameRate>().unwrap().millihertz(),
        29_970
    );
    assert_eq!(long.parse::<FrameRate>().unwrap().millihertz(), 29_970);
    assert_eq!(
        "29.9995000000000000000000000000000000000000000001"
            .parse::<FrameRate>()
            .unwrap()
            .millihertz(),
        30_000
    );
    assert_eq!(
        "000000000000000000000000000000000000000000000025"
            .parse::<FrameRate>()
            .unwrap()
            .millihertz(),
        25_000
    );
    let text = format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,FRAME-RATE={long}\nv.m3u8\n");
    let p = MasterPlaylist::parse(&text).unwrap();
    assert_eq!(p.variants[0].frame_rate.unwrap().millihertz(), 29_970);
}

#[test]
fn frame_rate_still_rejects_malformed_and_overflowing_values() {
    for bad in ["", ".5", "25.", "-25", "2x", "99999999999"] {
        assert!(bad.parse::<FrameRate>().is_err(), "{bad:?}");
    }
}

fn key_playlist(key: &str) -> String {
    format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n{key}\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n")
}

#[test]
fn method_none_with_other_attributes_is_rejected() {
    for bad in [
        "#EXT-X-KEY:METHOD=NONE,URI=\"k\"",
        "#EXT-X-KEY:METHOD=NONE,IV=0x000102030405060708090A0B0C0D0E0F",
        "#EXT-X-KEY:METHOD=NONE,KEYFORMAT=\"identity\"",
    ] {
        let p = MediaPlaylist::parse(&key_playlist(bad)).unwrap();
        assert!(p.segment_keys().is_err(), "{bad}");
    }
    let p = MediaPlaylist::parse(&key_playlist("#EXT-X-KEY:METHOD=NONE")).unwrap();
    assert_eq!(p.segment_keys().unwrap(), vec![None]);
}
