//! dvr-runtime PR 1 additions to `broadcast-hls` (issue #3925):
//! typed `EXT-X-DATERANGE`, custom tags, segment-scoped parse options,
//! incremental render, and the dangling-segment-tag diagnostic.
//!
//! Fixtures are loaded only through `test_fixtures`; every path here is a
//! committed file, so a missing one is a hard failure, never a skip.

use broadcast_hls::{DateRange, Error};

fn daterange_lines(name: &str) -> Vec<String> {
    test_fixtures::read_to_string(name)
        .lines()
        .filter(|line| line.starts_with("#EXT-X-DATERANGE:"))
        .map(str::to_string)
        .collect()
}

/// Every committed DATERANGE line this crate ships as a test vector parses
/// typed and re-renders byte-identically — including the two wire spellings
/// of the same instant (`...:00Z` and `...:00.000Z`), which is why
/// `DateRange` retains the date text.
#[test]
fn committed_daterange_lines_round_trip_byte_identically() {
    let fixtures = [
        "hls/daterange/daterange_2002.txt",
        "hls/daterange/daterange_2004.txt",
        "hls/handbuilt/daterange-scte35-media.m3u8",
        "golden/hls/daterange.txt",
    ];
    let mut seen = 0usize;
    for fixture in fixtures {
        let lines = daterange_lines(fixture);
        assert!(!lines.is_empty(), "{fixture} must contain a DATERANGE line");
        for line in lines {
            let range = DateRange::parse_tag_line(&line)
                .unwrap_or_else(|e| panic!("{fixture}: {line}\n  must parse: {e}"));
            let rendered = range
                .to_tag_line()
                .unwrap_or_else(|e| panic!("{fixture}: {line}\n  must render: {e}"));
            assert_eq!(rendered, line, "{fixture}: byte-identical round trip");
            seen += 1;
        }
    }
    assert_eq!(seen, 8, "the committed fixtures carry eight DATERANGE lines");
}

/// The typed fields are actually populated, not just swept into `extra_attrs`
/// (which would make the round trip above pass vacuously).
#[test]
fn daterange_fields_are_typed() {
    let line = daterange_lines("hls/daterange/daterange_2002.txt")
        .into_iter()
        .next()
        .expect("fixture line");
    let range = DateRange::parse_tag_line(&line).expect("must parse");
    assert_eq!(range.id, "2002");
    assert_eq!(range.class, None);
    let start = range.start.expect("START-DATE must be typed");
    assert_eq!(start.render(), "2018-10-29T10:38:00.000Z");
    assert_eq!(range.planned_duration.map(|d| d.get()), Some(24.0));
    assert!(!range.end_on_next);
    assert_eq!(range.extra_attrs.len(), 1);
    assert_eq!(range.extra_attrs[0].0, "SCTE35-OUT");
    assert!(range.extra_attrs[0].1.as_str().starts_with("0xFC3021"));
    assert!(range.client_attrs.is_empty());

    let design = "#EXT-X-DATERANGE:ID=\"programme-1537-30807\",CLASS=\"io.github.fishloa.rust-broadcast.programme\",START-DATE=\"2024-05-18T18:00:02.000Z\",PLANNED-DURATION=3600,END-ON-NEXT=YES,X-DVB-SERVICE-ID=1537,X-DVB-EVENT-ID=30807,X-SCHEDULED-START=\"2024-05-18T18:00:00Z\",X-TITLE=\"50' inside\"";
    let range = DateRange::parse_tag_line(design).expect("design DATERANGE must parse");
    assert_eq!(range.client_attrs.len(), 4);
    assert!(range.client_attrs.iter().any(|(n, v)| n.as_str() == "X-TITLE" && v.as_str() == "50' inside"));
    assert_eq!(range.to_tag_line().expect("render"), design);
}

/// The locally checkable §4.4.5.1 rules are typed errors, not rendered tags.
#[test]
fn daterange_local_rules_are_typed_errors() {
    let no_class = "#EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2024-01-01T00:00:00Z\",END-ON-NEXT=YES";
    assert!(matches!(
        DateRange::parse_tag_line(no_class),
        Err(Error::InvalidDateRange { .. })
    ));

    let with_duration = "#EXT-X-DATERANGE:ID=\"a\",CLASS=\"c\",START-DATE=\"2024-01-01T00:00:00Z\",DURATION=1,END-ON-NEXT=YES";
    assert!(matches!(
        DateRange::parse_tag_line(with_duration),
        Err(Error::InvalidDateRange { .. })
    ));

    let before = "#EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2024-01-01T00:00:02Z\",END-DATE=\"2024-01-01T00:00:01Z\"";
    assert!(matches!(
        DateRange::parse_tag_line(before),
        Err(Error::InvalidDateRange { .. })
    ));
}

/// An `X-` name that is not a legal `AttributeName` is a typed error.
#[test]
fn invalid_client_attribute_names_are_typed_errors() {
    use broadcast_hls::ClientAttrName;
    assert!(matches!(
        ClientAttrName::new("DVB-SERVICE-ID"),
        Err(Error::InvalidClientAttrName { .. })
    ));
    assert!(matches!(
        ClientAttrName::new("X-lower-case"),
        Err(Error::InvalidClientAttrName { .. })
    ));
    assert!(ClientAttrName::new("X-DVB-SERVICE-ID").is_ok());
}

/// The two vendor tags the DVR archive writes round-trip through
/// `CustomTag`, attribute order included.
#[test]
fn custom_tag_round_trips_wire_order() {
    use broadcast_hls::CustomTag;
    let archive = "#EXT-X-RUSTBROADCAST-ARCHIVE:VERSION=1,CONTAINER=FMP4,PERIOD=4";
    let tag = CustomTag::parse_tag_line(archive).expect("must parse");
    assert_eq!(tag.name.as_str(), "EXT-X-RUSTBROADCAST-ARCHIVE");
    assert_eq!(
        tag.attrs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["VERSION", "CONTAINER", "PERIOD"],
        "wire order must survive"
    );
    assert_eq!(tag.to_tag_line().expect("render"), archive);

    let timing =
        "#EXT-X-RUSTBROADCAST-TIMING:SEQ=121,DTS=242000000000,DUR=2000000000,TIMESCALE=1000000000";
    let tag = CustomTag::parse_tag_line(timing).expect("must parse");
    assert_eq!(tag.to_tag_line().expect("render"), timing);

    let bare = CustomTag::parse_tag_line("#EXT-X-RUSTBROADCAST-ARCHIVE").expect("must parse");
    assert!(bare.attrs.is_empty());
    assert_eq!(bare.to_tag_line().expect("render"), "#EXT-X-RUSTBROADCAST-ARCHIVE");
}

/// A tag name that is not `EXT-` (or is not from the tag character set) is a
/// typed error.
#[test]
fn invalid_custom_tag_names_are_typed_errors() {
    use broadcast_hls::{CustomTag, CustomTagName};
    assert!(matches!(
        CustomTagName::new("X-RUSTBROADCAST"),
        Err(Error::InvalidCustomTagName { .. })
    ));
    assert!(matches!(
        CustomTagName::new("EXT-x-lower"),
        Err(Error::InvalidCustomTagName { .. })
    ));
    assert!(matches!(
        CustomTag::parse_tag_line("#EXT-X-BAD:lower=1"),
        Err(Error::InvalidAttributeName { .. })
    ));
    assert!(CustomTagName::new("EXT-X-RUSTBROADCAST-TIMING").is_ok());
}

const DESIGN_PLAYLIST: &str = "hls/handbuilt/dvr-runtime-design.m3u8";

fn segment_scoped_options() -> broadcast_hls::ParseOptions {
    use broadcast_hls::CustomTagName;
    broadcast_hls::ParseOptions::default().with_segment_scoped_tag(
        CustomTagName::new("EXT-X-RUSTBROADCAST-TIMING").expect("valid tag name"),
    )
}

/// The design's §3.3 period playlist parses with options and re-renders
/// byte-identically, with the per-segment timing tag still on its segment.
#[test]
fn design_playlist_round_trips_byte_identically_with_segment_scoped_timing() {
    use broadcast_hls::{MediaPlaylist, limits::Limits};

    let input = test_fixtures::read_to_string(DESIGN_PLAYLIST);
    let options = segment_scoped_options();
    let playlist = MediaPlaylist::parse_with_options(&input, &Limits::default(), &options)
        .unwrap_or_else(|e| panic!("design playlist must parse: {e}"));

    assert_eq!(playlist.segments.len(), 2);
    assert_eq!(
        playlist
            .segments[0]
            .pre_tags
            .iter()
            .filter(|t| t.starts_with("#EXT-X-RUSTBROADCAST-TIMING:"))
            .count(),
        1,
        "first segment carries its timing tag"
    );
    assert_eq!(
        playlist
            .segments[1]
            .pre_tags
            .iter()
            .filter(|t| t.starts_with("#EXT-X-RUSTBROADCAST-TIMING:"))
            .count(),
        1,
        "second segment carries its own timing tag"
    );
    assert!(
        playlist
            .extra_tags
            .iter()
            .all(|t| !t.starts_with("#EXT-X-RUSTBROADCAST-TIMING:")),
        "a declared segment-scoped tag must not land at playlist level"
    );
    assert!(
        playlist
            .extra_tags
            .iter()
            .any(|t| t == "#EXT-X-RUSTBROADCAST-ARCHIVE:VERSION=1,CONTAINER=FMP4,PERIOD=4"),
        "the playlist-level archive marker stays in the header"
    );

    assert_eq!(playlist.to_m3u8().expect("render"), input);
}

/// Bite for H3: `parse` (default options) puts the timing tag at playlist
/// level instead, so the option is what keeps it with its segment.
#[test]
fn default_parse_still_hoists_the_timing_tag() {
    use broadcast_hls::MediaPlaylist;

    let input = test_fixtures::read_to_string(DESIGN_PLAYLIST);
    let playlist = MediaPlaylist::parse(&input).expect("must parse");
    assert!(
        playlist
            .extra_tags
            .iter()
            .any(|t| t.starts_with("#EXT-X-RUSTBROADCAST-TIMING:")),
        "without the option an unknown tag is playlist-level"
    );
    assert!(
        playlist
            .segments
            .iter()
            .all(|s| s.pre_tags.iter().all(|t| !t.starts_with("#EXT-X-RUSTBROADCAST-TIMING:")))
    );
}

fn assert_render_split_matches(playlist: &broadcast_hls::MediaPlaylist) {
    let mut split = playlist.render_header().expect("header");
    for index in 0..playlist.segments.len() {
        split.push_str(&playlist.render_segment(index).expect("segment"));
    }
    split.push_str(&playlist.render_endlist().expect("endlist"));
    assert_eq!(
        split,
        playlist.to_m3u8().expect("full render"),
        "header + segments + endlist must equal to_m3u8"
    );
}

/// H4: the render split is the full render, over every committed media
/// playlist fixture (LL-HLS trailing reports included) and generated shapes.
#[test]
fn render_split_equals_full_render() {
    use broadcast_hls::{MediaPlaylist, limits::Limits};

    let fixtures = [
        "hls/valid.m3u8",
        "hls/reference-vod.m3u8",
        "hls/bad-extinf.m3u8",
        "hls/malformed-daterange.m3u8",
        "hls/byterange-hls/index.m3u8",
        "hls/ts-hls/index.m3u8",
        "hls/handbuilt/daterange-scte35-media.m3u8",
        "hls/handbuilt/live-vod-gap-bitrate.m3u8",
        "hls/handbuilt/low-latency-parts-preload-report.m3u8",
        DESIGN_PLAYLIST,
        "hls/real/bipbop-fmp4-hevc/a1-audio.m3u8",
        "hls/real/bipbop-fmp4-hevc/v5-video.m3u8",
        "hls/real/bipbop-ts/alternate-audio.m3u8",
        "hls/real/bipbop-ts/gear1-video.m3u8",
    ];
    let mut checked = 0usize;
    for fixture in fixtures {
        let input = test_fixtures::read_to_string(fixture);
        let playlist = MediaPlaylist::parse_with_options(&input, &Limits::default(), &segment_scoped_options())
            .unwrap_or_else(|e| panic!("{fixture} must parse: {e}"));
        assert_render_split_matches(&playlist);
        checked += 1;
    }
    assert_eq!(checked, fixtures.len());

    // A closed playlist with no segments, an open (endlist-less) one, and a
    // programmatically built segment record.
    let empty = MediaPlaylist {
        target_duration: 2,
        ..Default::default()
    };
    assert_render_split_matches(&empty);
    assert!(empty.render_endlist().expect("endlist").is_empty());

    let mut open = MediaPlaylist {
        target_duration: 4,
        ..Default::default()
    };
    open.segments.push(broadcast_hls::MediaSegment {
        uri: "seg0.m4s".into(),
        duration: broadcast_hls::DecimalSeconds::new(4.0).expect("valid"),
        ..Default::default()
    });
    assert_render_split_matches(&open);
    assert!(open.render_endlist().expect("endlist").is_empty());

    let mut closed = open.clone();
    closed.endlist = true;
    assert_eq!(closed.render_endlist().expect("endlist"), "#EXT-X-ENDLIST\n");
    assert_render_split_matches(&closed);
}

/// H4: an index the playlist does not have is a typed error, not a panic.
#[test]
fn render_segment_out_of_range_is_a_typed_error() {
    use broadcast_hls::MediaPlaylist;
    let playlist = MediaPlaylist {
        target_duration: 2,
        ..Default::default()
    };
    assert!(matches!(
        playlist.render_segment(0),
        Err(Error::SegmentIndexOutOfRange { index: 0, len: 0 })
    ));
}
