use broadcast_hls::{AttrValue, MasterPlaylist, Variant, VideoRange};

#[test]
fn from_transfer_characteristics_mapping_table() {
    // Independent hand-written expectation (not the production table).
    fn expected(code: u8) -> Option<VideoRange> {
        match code {
            1 | 6 | 13 | 14 | 15 => Some(VideoRange::Sdr),
            16 => Some(VideoRange::Pq),
            18 => Some(VideoRange::Hlg),
            _ => None,
        }
    }
    for code in 0u8..=255 {
        assert_eq!(
            VideoRange::from_transfer_characteristics(code),
            expected(code),
            "mismatch at code {code}"
        );
    }
    // name() tokens are exactly the wire strings.
    assert_eq!(VideoRange::Sdr.name(), "SDR");
    assert_eq!(VideoRange::Hlg.name(), "HLG");
    assert_eq!(VideoRange::Pq.name(), "PQ");
    // attr() produces a bare VIDEO-RANGE attribute.
    assert_eq!(
        VideoRange::Pq.attr(),
        ("VIDEO-RANGE".to_string(), AttrValue::bare("PQ").unwrap())
    );
}

#[test]
fn video_range_round_trips_bare_unquoted() {
    for range in [VideoRange::Hlg, VideoRange::Pq] {
        let master = MasterPlaylist {
            media: vec![],
            variants: vec![Variant {
                bandwidth: 1000,
                average_bandwidth: None,
                frame_rate: None,
                uri: "v.m3u8".to_string(),
                audio: None,
                video: None,
                subtitles: None,
                closed_captions: None,
                extra_attrs: vec![range.attr()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let rendered = master.to_m3u8().unwrap();
        let token = range.name();
        assert!(
            rendered.contains(&format!("VIDEO-RANGE={token}")),
            "rendered missing bare VIDEO-RANGE={token}:\n{rendered}"
        );
        assert!(
            !rendered.contains(&format!("VIDEO-RANGE=\"{token}\"")),
            "VIDEO-RANGE must be bare, got:\n{rendered}"
        );
        let reparsed = MasterPlaylist::parse(&rendered).unwrap();
        assert_eq!(reparsed.variants[0].extra_attrs, vec![range.attr()]);
    }
}
