//! LL-HLS builder methods, sequence-number conversions and `Display` impls,
//! each pinned by the exact rendered or formatted value (audit 21 finding 44,
//! issue #2939).

use broadcast_hls::{
    ByteRange, DecimalSeconds, LowLatencyConfig, MapTag, MediaPlaylist, MediaSegment,
    MediaSequence, PartIndex, PartSpec, PreloadHintType, RenditionReport, SessionDataFormat,
    SignedDecimalSeconds,
};

fn secs(value: f64) -> DecimalSeconds {
    DecimalSeconds::new(value).unwrap()
}

#[test]
fn low_latency_config_builders_render_their_attributes() {
    let config = LowLatencyConfig::default()
        .with_part_target(secs(0.334))
        .with_part_hold_back(secs(1.002))
        .with_can_skip_until(Some(secs(24.0)))
        .with_can_skip_dateranges(true)
        .with_can_block_reload(true)
        .with_hold_back(Some(secs(8.0)))
        .with_preload_hint_part("next.m4s".to_string())
        .with_preload_hint_type(PreloadHintType::Part);
    let pl = MediaPlaylist {
        target_duration: 2,
        low_latency: Some(config),
        segments: vec![MediaSegment {
            uri: "s0.m4s".into(),
            duration: secs(2.0),
            parts: vec![
                PartSpec::new("s0.0.m4s".to_string(), secs(0.334))
                    .with_independent(true)
                    .with_byte_range(ByteRange {
                        length: 500,
                        offset: Some(100),
                    })
                    .with_gap(true),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let out = pl.to_m3u8().unwrap();
    assert!(
        out.contains("#EXT-X-PART-INF:PART-TARGET=0.334\n"),
        "with_part_target: {out}"
    );
    assert!(
        out.contains(
            "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.002,HOLD-BACK=8,\
CAN-SKIP-UNTIL=24,CAN-SKIP-DATERANGES=YES\n"
        ),
        "with_can_block_reload/part_hold_back/hold_back/can_skip_until/can_skip_dateranges: {out}"
    );
    assert!(
        out.contains("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"next.m4s\"\n"),
        "with_preload_hint_part/type: {out}"
    );
    assert!(
        out.contains(
            "#EXT-X-PART:DURATION=0.334,URI=\"s0.0.m4s\",BYTERANGE=\"500@100\",\
INDEPENDENT=YES,GAP=YES\n"
        ),
        "PartSpec with_independent/with_byte_range/with_gap: {out}"
    );

    let map_hint = MediaPlaylist {
        target_duration: 2,
        low_latency: Some(
            LowLatencyConfig::default()
                .with_preload_hint_part("init.mp4".to_string())
                .with_preload_hint_type(PreloadHintType::Map),
        ),
        ..Default::default()
    };
    assert!(
        map_hint
            .to_m3u8()
            .unwrap()
            .contains("#EXT-X-PRELOAD-HINT:TYPE=MAP,URI=\"init.mp4\"\n")
    );
}

#[test]
fn rendition_report_with_last_part_renders_last_part() {
    let pl = MediaPlaylist {
        target_duration: 2,
        rendition_reports: vec![
            RenditionReport::new("r.m3u8".to_string(), MediaSequence::new(7))
                .with_last_part(PartIndex::new(3)),
        ],
        ..Default::default()
    };
    let out = pl.to_m3u8().unwrap();
    assert!(
        out.contains("#EXT-X-RENDITION-REPORT:URI=\"r.m3u8\",LAST-MSN=7,LAST-PART=3\n"),
        "{out}"
    );
}

#[test]
fn map_is_rendered_exactly_once_per_change_not_once_per_segment() {
    let a = MapTag {
        uri: "a.mp4".to_string(),
        byte_range: None,
        extra_attrs: Vec::new(),
    };
    let b = MapTag {
        uri: "b.mp4".to_string(),
        byte_range: None,
        extra_attrs: Vec::new(),
    };
    let pl = MediaPlaylist {
        target_duration: 2,
        segments: vec![
            MediaSegment {
                uri: "s0.m4s".into(),
                duration: secs(2.0),
                map: Some(a.clone()),
                ..Default::default()
            },
            MediaSegment {
                uri: "s1.m4s".into(),
                duration: secs(2.0),
                map: Some(a),
                ..Default::default()
            },
            MediaSegment {
                uri: "s2.m4s".into(),
                duration: secs(2.0),
                map: Some(b),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let out = pl.to_m3u8().unwrap();
    assert_eq!(
        out.matches("#EXT-X-MAP:").count(),
        2,
        "EXT-X-MAP applies until it changes, so two maps render twice: {out}"
    );
    assert!(out.contains("#EXT-X-MAP:URI=\"a.mp4\"\n"));
    assert!(out.contains("#EXT-X-MAP:URI=\"b.mp4\"\n"));
}

#[test]
fn sequence_numbers_convert_and_report_their_value() {
    use core::num::NonZeroU32;

    assert_eq!(MediaSequence::new(7).value(), 7);
    assert_eq!(MediaSequence::ZERO.value(), 0);
    assert_eq!(MediaSequence::from(266u64).value(), 266);
    assert_eq!(u64::from(MediaSequence::new(266)), 266);
    assert_eq!(format!("{}", MediaSequence::new(266)), "266");

    assert_eq!(PartIndex::new(3).value(), 3);
    assert_eq!(PartIndex::ZERO.value(), 0);
    assert_eq!(PartIndex::from(5u32).value(), 5);
    assert_eq!(u32::from(PartIndex::new(5)), 5);
    assert_eq!(PartIndex::from(NonZeroU32::new(9).unwrap()).value(), 9);
    assert_eq!(format!("{}", PartIndex::new(3)), "3");
}

#[cfg(feature = "serde")]
#[test]
fn sequence_numbers_serde_is_the_bare_wire_number() {
    assert_eq!(serde_json::to_string(&MediaSequence::new(7)).unwrap(), "7");
    assert_eq!(
        serde_json::from_str::<MediaSequence>("7").unwrap(),
        MediaSequence::new(7)
    );
    assert_eq!(serde_json::to_string(&PartIndex::new(3)).unwrap(), "3");
    assert_eq!(
        serde_json::from_str::<PartIndex>("3").unwrap(),
        PartIndex::new(3)
    );
}

#[test]
fn seconds_display_matches_the_wire_spelling() {
    assert_eq!(format!("{}", DecimalSeconds::new(2.5).unwrap()), "2.5");
    assert_eq!(format!("{}", DecimalSeconds::new(0.334).unwrap()), "0.334");
    assert_eq!(
        format!("{}", SignedDecimalSeconds::new(-0.5).unwrap()),
        "-0.5"
    );
    assert_eq!(
        format!("{}", SignedDecimalSeconds::new(10.25).unwrap()),
        "10.25"
    );
}

#[test]
fn session_data_format_names_are_the_spec_tokens() {
    assert_eq!(SessionDataFormat::Json.name(), "JSON");
    assert_eq!(SessionDataFormat::Raw.name(), "RAW");
    assert_eq!(format!("{}", SessionDataFormat::Raw), "RAW");
    assert_eq!(format!("{}", SessionDataFormat::Json), "JSON");
}
