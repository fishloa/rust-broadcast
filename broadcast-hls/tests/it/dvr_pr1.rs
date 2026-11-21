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
    assert_eq!(
        seen, 8,
        "the committed fixtures carry eight DATERANGE lines"
    );
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
    assert!(
        range
            .client_attrs
            .iter()
            .any(|(n, v)| n.as_str() == "X-TITLE" && v.as_str() == "50' inside")
    );
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
        tag.attrs
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
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
    assert_eq!(
        bare.to_tag_line().expect("render"),
        "#EXT-X-RUSTBROADCAST-ARCHIVE"
    );
}

/// A tag name that is not `EXT-` (or is not from the tag character set) is a
/// typed error.
#[test]
fn invalid_custom_tag_names_are_typed_errors() {
    use broadcast_hls::{CustomTag, CustomTagName, CustomTagNameError};
    assert!(matches!(
        CustomTagName::new("X-RUSTBROADCAST"),
        Err(Error::InvalidCustomTagName {
            reason: CustomTagNameError::MissingPrefix,
            ..
        })
    ));
    assert!(matches!(
        CustomTagName::new("EXT-x-lower"),
        Err(Error::InvalidCustomTagName {
            reason: CustomTagNameError::InvalidCharacter,
            ..
        })
    ));
    assert!(matches!(
        CustomTagName::new("EXT--"),
        Err(Error::InvalidCustomTagName {
            reason: CustomTagNameError::EmptyName,
            ..
        })
    ));
    assert!(matches!(
        CustomTagName::new("EXT-X-"),
        Err(Error::InvalidCustomTagName {
            reason: CustomTagNameError::TrailingHyphen,
            ..
        })
    ));
    assert!(matches!(
        CustomTagName::new("EXT-X-MAP"),
        Err(Error::InvalidCustomTagName {
            reason: CustomTagNameError::StandardTag,
            ..
        })
    ));
    assert!(matches!(
        CustomTag::parse_tag_line("#EXT-X-BAD:lower=1"),
        Err(Error::InvalidAttributeList {
            reason: broadcast_hls::AttributeListError::InvalidName,
            ..
        })
    ));
    assert!(CustomTagName::new("EXT-X-RUSTBROADCAST-TIMING").is_ok());
}

fn daterange_roundtrip(line: &str) {
    let range =
        DateRange::parse_tag_line(line).unwrap_or_else(|e| panic!("{line}\n  must parse: {e}"));
    assert_eq!(
        range.to_tag_line().expect("render"),
        line,
        "byte-identical round trip"
    );
}

/// Item 2: wire order, quoting and decimal text survive; anything outside the
/// §4.2 grammar, or a known attribute with the wrong value kind, is a typed
/// rejection.
#[test]
fn daterange_wire_grammar_is_strict_and_lossless() {
    use broadcast_hls::{AttributeListError, DateRangeError};

    for line in [
        // START-DATE before ID keeps its position.
        "#EXT-X-DATERANGE:START-DATE=\"2024-01-01T00:00:00Z\",ID=\"a\"",
        // A client attribute between two known ones keeps its position.
        "#EXT-X-DATERANGE:ID=\"a\",X-A=1,CLASS=\"c\"",
        // An SCTE35 attribute before a client attribute keeps its position.
        "#EXT-X-DATERANGE:ID=\"a\",SCTE35-OUT=0xFC,X-A=1",
        // Decimal text is not normalised.
        "#EXT-X-DATERANGE:ID=\"a\",DURATION=10.0",
        "#EXT-X-DATERANGE:ID=\"a\",DURATION=10",
        "#EXT-X-DATERANGE:ID=\"a\",CUE=\"ONCE,PRE\"",
        "#EXT-X-DATERANGE:ID=\"a\",X-Q=\"a,b\"",
        "#EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2024-01-01T00:00:00Z\",END-DATE=\"2024-01-01T00:00:10Z\",DURATION=10",
    ] {
        daterange_roundtrip(line);
    }

    for (line, ok) in [
        (
            "#EXT-X-DATERANGE:ID=a",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=a"),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::WrongAttributeKind,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\", START-DATE=\"2024-01-01T00:00:00Z\"",
            matches!(
                DateRange::parse_tag_line(
                    "#EXT-X-DATERANGE:ID=\"a\", START-DATE=\"2024-01-01T00:00:00Z\""
                ),
                Err(Error::InvalidAttributeList {
                    reason: AttributeListError::Whitespace,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",FOO",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",FOO"),
                Err(Error::InvalidAttributeList {
                    reason: AttributeListError::MissingEquals,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:"),
                Err(Error::InvalidAttributeList {
                    reason: AttributeListError::Empty,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"\"",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"\""),
                Err(Error::InvalidAttributeList {
                    reason: AttributeListError::EmptyQuotedValue,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",DURATION=\"10\"",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",DURATION=\"10\""),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::InvalidDurationValue,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",X-Q=foo",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",X-Q=foo"),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::InvalidClientAttributeValue,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",X-Q=0xZZ",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",X-Q=0xZZ"),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::InvalidClientAttributeValue,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",SCTE35-OUT=12",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",SCTE35-OUT=12"),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::InvalidScte35Value,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",END-ON-NEXT=NO",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",END-ON-NEXT=NO"),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::EndOnNextNotYes,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",CUE=\"PRE,POST\"",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",CUE=\"PRE,POST\""),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::CuePreAndPost,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",CUE=\"BOGUS\"",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",CUE=\"BOGUS\""),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::InvalidCue,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",CUE=PRE",
            matches!(
                DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",CUE=PRE"),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::WrongAttributeKind,
                    ..
                })
            ),
        ),
        (
            "#EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2024-01-01T00:00:00Z\",END-DATE=\"2024-01-01T00:00:09Z\",DURATION=10",
            matches!(
                DateRange::parse_tag_line(
                    "#EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2024-01-01T00:00:00Z\",END-DATE=\"2024-01-01T00:00:09Z\",DURATION=10"
                ),
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::EndDateDurationMismatch,
                    ..
                })
            ),
        ),
    ] {
        assert!(ok, "wrong rejection for {line}");
    }

    let cue =
        DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",CUE=\"PRE,ONCE\"").expect("valid CUE");
    let cue = cue.cue.expect("typed cue");
    assert_eq!(cue.triggers.len(), 2);
    assert_eq!(cue.triggers[0].name(), "PRE");
    assert_eq!(cue.triggers[0].to_string(), "PRE");
    assert_eq!(cue.render(), "PRE,ONCE");

    assert!(matches!(
        DateRange::new(""),
        Err(Error::InvalidDateRange {
            reason: DateRangeError::EmptyId,
            ..
        })
    ));
    assert!(matches!(
        DateRange::new("a").expect("valid id").to_tag_line(),
        Ok(line) if line == "#EXT-X-DATERANGE:ID=\"a\""
    ));
}

/// Item 2: the custom-tag scanner rejects the same grammar violations instead
/// of silently dropping the attribute list.
#[test]
fn custom_tag_wire_grammar_is_strict() {
    use broadcast_hls::{AttributeListError, CustomTag};

    for line in [
        "#EXT-X-FOO:A=1,B=\"2\"",
        "#EXT-X-FOO",
        "#EXT-X-FOO:A=\"a,b\",B=2",
    ] {
        let tag = CustomTag::parse_tag_line(line).expect("must parse");
        assert_eq!(tag.to_tag_line().expect("render"), line);
    }

    for (line, reason) in [
        ("#EXT-X-FOO:", AttributeListError::Empty),
        ("#EXT-X-FOO:123", AttributeListError::MissingEquals),
        ("#EXT-X-FOO:A=1,B", AttributeListError::MissingEquals),
        (
            "#EXT-X-FOO:A=\"1\"x",
            AttributeListError::TrailingCharacters,
        ),
        ("#EXT-X-FOO:A=1,", AttributeListError::TrailingComma),
    ] {
        assert!(
            matches!(
                CustomTag::parse_tag_line(line),
                Err(Error::InvalidAttributeList { reason: got, .. }) if got == reason
            ),
            "{line} must be rejected as {reason:?}"
        );
    }
}

/// Item 1: `render_header` validates the fields it emits itself (a `skip` id
/// cannot inject a line), and `render_segment` the same for its title.
#[test]
fn incremental_renderers_validate_what_they_emit() {
    use broadcast_hls::{MediaPlaylist, MediaSegment, SkipInfo};

    let mut skip = SkipInfo::default();
    skip.recently_removed_daterange_ids
        .push("x\"\n#EXT-X-ENDLIST\n#Y".into());
    let header = MediaPlaylist {
        target_duration: 2,
        skip: Some(skip),
        ..Default::default()
    };
    assert!(matches!(
        header.render_header(),
        Err(Error::InvalidQuotedString { .. })
    ));
    assert!(matches!(
        header.to_m3u8(),
        Err(Error::InvalidQuotedString { .. })
    ));

    let segment = MediaPlaylist {
        target_duration: 2,
        segments: vec![MediaSegment {
            uri: "seg0.ts".into(),
            duration: broadcast_hls::DecimalSeconds::new(2.0).expect("valid"),
            title: Some("x\"\n#EXT-X-ENDLIST\n#Y".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(matches!(
        segment.render_segment(0),
        Err(Error::InvalidQuotedString { .. })
    ));
    assert!(matches!(
        segment.to_m3u8(),
        Err(Error::InvalidQuotedString { .. })
    ));
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
        playlist.segments[0]
            .pre_tags
            .iter()
            .filter(|t| t.starts_with("#EXT-X-RUSTBROADCAST-TIMING:"))
            .count(),
        1,
        "first segment carries its timing tag"
    );
    assert_eq!(
        playlist.segments[1]
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
    assert!(playlist.segments.iter().all(|s| {
        s.pre_tags
            .iter()
            .all(|t| !t.starts_with("#EXT-X-RUSTBROADCAST-TIMING:"))
    }));
}

fn pinned_version(playlist: &broadcast_hls::MediaPlaylist) -> u8 {
    playlist
        .version
        .max(playlist.computed_version().unwrap_or(0))
}

/// The real append property: with `version` pinned, the header and every
/// already-written segment record are unchanged by appending more segments.
fn assert_append_prefix(playlist: &broadcast_hls::MediaPlaylist) {
    let mut full = playlist.clone();
    full.version = pinned_version(playlist);
    let header = full.render_header().expect("full header");
    let segments: Vec<String> = (0..full.segments.len())
        .map(|i| full.render_segment(i).expect("full segment"))
        .collect();

    for k in 0..=full.segments.len() {
        let mut prefix = full.clone();
        prefix.segments.truncate(k);
        assert_eq!(
            prefix.render_header().expect("prefix header"),
            header,
            "header changed after truncating to {k} segments"
        );
        for (i, expected) in segments.iter().enumerate().take(k) {
            assert_eq!(
                &prefix.render_segment(i).expect("prefix segment"),
                expected,
                "segment {i} changed after truncating to {k} segments"
            );
        }
    }
}

/// H4: the append property over every committed media playlist and generated
/// shapes. A field other than `version` that makes the header depend on later
/// segments fails here.
#[test]
fn render_prefix_is_stable_as_segments_are_appended() {
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
    for fixture in fixtures {
        let input = test_fixtures::read_to_string(fixture);
        let playlist = MediaPlaylist::parse_with_options(
            &input,
            &Limits::default(),
            &segment_scoped_options(),
        )
        .unwrap_or_else(|e| panic!("{fixture} must parse: {e}"));
        assert_append_prefix(&playlist);
    }

    let empty = MediaPlaylist {
        target_duration: 2,
        ..Default::default()
    };
    assert_append_prefix(&empty);
    assert!(empty.render_endlist().expect("endlist").is_empty());

    let mut generated = MediaPlaylist {
        target_duration: 4,
        ..Default::default()
    };
    for i in 0..3 {
        generated.segments.push(broadcast_hls::MediaSegment {
            uri: format!("seg{i}.m4s"),
            duration: broadcast_hls::DecimalSeconds::new(4.0).expect("valid"),
            discontinuous: i == 2,
            byte_range: Some(broadcast_hls::ByteRange {
                length: 100,
                offset: Some(812 + i * 100),
            }),
            ..Default::default()
        });
    }
    generated.endlist = true;
    assert_append_prefix(&generated);
    assert_eq!(
        generated.render_endlist().expect("endlist"),
        "#EXT-X-ENDLIST\n"
    );
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

/// H5: an `#EXTINF` with no URI line before end of input is reported, not
/// silently dropped.
#[test]
fn dangling_extinf_is_one_diagnostic() {
    use broadcast_hls::{MediaPlaylist, ParseDiagnosticKind};

    let input = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\n";
    let playlist = MediaPlaylist::parse(input).expect("must parse");
    assert!(playlist.segments.is_empty());
    assert_eq!(playlist.diagnostics.len(), 1, "{:?}", playlist.diagnostics);
    assert_eq!(
        playlist.diagnostics[0].kind,
        ParseDiagnosticKind::DanglingSegmentTags
    );
    assert_eq!(playlist.diagnostics[0].line_no, 3);
    assert_eq!(playlist.diagnostics[0].line, "#EXTINF:2,");
    assert_eq!(
        ParseDiagnosticKind::DanglingSegmentTags.to_string(),
        "dangling_segment_tags"
    );
}

/// H5: a dangling `#EXT-X-BYTERANGE` at EOF is reported the same way.
#[test]
fn dangling_byterange_is_one_diagnostic() {
    use broadcast_hls::{MediaPlaylist, ParseDiagnosticKind};

    let input = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\n#EXT-X-BYTERANGE:100@0\n";
    let playlist = MediaPlaylist::parse(input).expect("must parse");
    assert!(playlist.segments.is_empty());
    let dangling: Vec<_> = playlist
        .diagnostics
        .iter()
        .filter(|d| d.kind == ParseDiagnosticKind::DanglingSegmentTags)
        .collect();
    assert_eq!(
        dangling.len(),
        2,
        "EXTINF and BYTERANGE each report: {dangling:?}"
    );
    assert_eq!(dangling[0].line, "#EXTINF:2,");
    assert_eq!(dangling[1].line, "#EXT-X-BYTERANGE:100@0");

    let only_range = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-BYTERANGE:100@0\n";
    let playlist = MediaPlaylist::parse(only_range).expect("must parse");
    assert_eq!(playlist.diagnostics.len(), 1, "{:?}", playlist.diagnostics);
    assert_eq!(playlist.diagnostics[0].line, "#EXT-X-BYTERANGE:100@0");

    let complete = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nseg0.ts\n";
    assert!(
        MediaPlaylist::parse(complete)
            .expect("must parse")
            .diagnostics
            .is_empty()
    );
}
