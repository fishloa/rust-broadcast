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
    assert_eq!(range.id(), "2002");
    assert_eq!(range.class(), None);
    let start = range.start().expect("START-DATE must be typed");
    assert_eq!(start.render(), "2018-10-29T10:38:00.000Z");
    assert_eq!(range.planned_duration().map(|d| d.get()), Some(24.0));
    assert!(!range.end_on_next());
    assert_eq!(range.extra_attrs().len(), 1);
    assert_eq!(range.extra_attrs()[0].0, "SCTE35-OUT");
    assert!(range.extra_attrs()[0].1.as_str().starts_with("0xFC3021"));
    assert!(range.client_attrs().is_empty());

    let design = "#EXT-X-DATERANGE:ID=\"programme-1537-30807\",CLASS=\"io.github.fishloa.rust-broadcast.programme\",START-DATE=\"2024-05-18T18:00:02.000Z\",PLANNED-DURATION=3600,END-ON-NEXT=YES,X-DVB-SERVICE-ID=1537,X-DVB-EVENT-ID=30807,X-SCHEDULED-START=\"2024-05-18T18:00:00Z\",X-TITLE=\"50' inside\"";
    let range = DateRange::parse_tag_line(design).expect("design DATERANGE must parse");
    assert_eq!(range.client_attrs().len(), 4);
    assert!(
        range
            .client_attrs()
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
    use broadcast_hls::{ClientAttrName, ClientAttrNameError};
    assert!(matches!(
        ClientAttrName::new("DVB-SERVICE-ID"),
        Err(Error::InvalidClientAttrName {
            reason: ClientAttrNameError::MissingPrefix,
            ..
        })
    ));
    assert!(matches!(
        ClientAttrName::new("X-lower-case"),
        Err(Error::InvalidClientAttrName {
            reason: ClientAttrNameError::InvalidCharacter,
            ..
        })
    ));
    assert!(matches!(
        ClientAttrName::new("X--"),
        Err(Error::InvalidClientAttrName {
            reason: ClientAttrNameError::EmptyName,
            ..
        })
    ));
    assert!(matches!(
        ClientAttrName::new("X-"),
        Err(Error::InvalidClientAttrName {
            reason: ClientAttrNameError::EmptyName,
            ..
        })
    ));
    assert!(matches!(
        ClientAttrName::new("X-DVB-"),
        Err(Error::InvalidClientAttrName {
            reason: ClientAttrNameError::TrailingHyphen,
            ..
        })
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
        tag.attrs()
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
    assert!(bare.attrs().is_empty());
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
                Err(Error::InvalidDateRange {
                    reason: DateRangeError::EmptyId,
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
    let cue = cue.cue().expect("typed cue");
    assert_eq!(cue.triggers().len(), 2);
    assert_eq!(cue.triggers()[0].name(), "PRE");
    assert_eq!(cue.triggers()[0].to_string(), "PRE");
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

/// Item 1: setters edit the single ordered attribute list. An existing
/// attribute keeps its position, a new one is appended, and equality is
/// semantic (not order or decimal spelling).
#[test]
fn daterange_edits_after_parse_are_rendered_in_place() {
    use broadcast_hls::{AttrValue, ClientAttrName, DecimalSeconds};

    let line = "#EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2024-01-01T00:00:00Z\",CLASS=\"c\"";
    let mut range = DateRange::parse_tag_line(line).expect("must parse");
    range.set_id("CHANGED").expect("set id");
    range.set_class(Some("d")).expect("set class");
    range
        .set_client_attr(
            ClientAttrName::new("X-NEW").expect("valid name"),
            AttrValue::bare("1").expect("valid value"),
        )
        .expect("set client attr");
    assert_eq!(
        range.to_tag_line().expect("render"),
        "#EXT-X-DATERANGE:ID=\"CHANGED\",START-DATE=\"2024-01-01T00:00:00Z\",CLASS=\"d\",X-NEW=1",
        "edits land in place; the new attribute is appended"
    );

    let parsed = DateRange::parse_tag_line(
        "#EXT-X-DATERANGE:START-DATE=\"2024-01-01T00:00:00Z\",ID=\"a\",DURATION=10.0",
    )
    .expect("must parse");
    let built = DateRange::new("a")
        .expect("valid id")
        .with_start(parsed.start().expect("start"))
        .with_duration(DecimalSeconds::new(10.0).expect("valid"));
    assert_eq!(parsed, built, "equality is semantic, not spelling or order");
    assert_eq!(
        built.to_tag_line().expect("render"),
        "#EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2024-01-01T00:00:00.000Z\",DURATION=10"
    );

    let unchanged = DateRange::parse_tag_line(line).expect("must parse");
    assert_eq!(unchanged.to_tag_line().expect("render"), line);
}

/// Item 1 (round 4): `Cue` is only constructible through its validator.
#[test]
fn cue_triggers_are_private_and_validated() {
    use broadcast_hls::{Cue, CueTrigger};

    assert!(Cue::new(Vec::new()).is_err());
    assert!(Cue::new(vec![CueTrigger::Pre, CueTrigger::Post]).is_err());
    assert!(Cue::new(vec![CueTrigger::Pre, CueTrigger::Pre]).is_err());
    let cue = Cue::new(vec![CueTrigger::Once]).expect("valid");
    assert_eq!(cue.triggers().len(), 1);
    assert_eq!(cue.triggers()[0], CueTrigger::Once);
}

/// Item 2 (round 4): a `CustomTag` setter enforces the wire rules, and
/// control characters are rejected everywhere a value is accepted.
#[test]
fn custom_tag_setters_and_control_characters_are_validated() {
    use broadcast_hls::{AttrValue, AttributeListError, CustomTag, CustomTagName};

    let mut tag = CustomTag::new(CustomTagName::new("EXT-X-FOO").expect("valid"));
    tag.set_attr("A", AttrValue::bare("1").expect("valid"))
        .expect("set");
    tag.set_attr("A", AttrValue::bare("2").expect("valid"))
        .expect("replace");
    assert_eq!(tag.attrs().len(), 1, "a repeated name replaces");
    assert_eq!(tag.to_tag_line().expect("render"), "#EXT-X-FOO:A=2");

    assert!(matches!(
        tag.set_attr("B", AttrValue::bare("").expect("empty bare value")),
        Err(Error::InvalidAttributeList {
            reason: AttributeListError::EmptyValue,
            ..
        })
    ));
    assert!(matches!(
        tag.set_attr("lower", AttrValue::quoted("x").expect("valid")),
        Err(Error::InvalidAttributeName { .. })
    ));
    assert!(matches!(
        CustomTag::parse_tag_line("#EXT-X-FOO:A=1,A=2"),
        Err(Error::DuplicateAttribute { .. })
    ));
    assert!(matches!(
        CustomTag::parse_tag_line("#EXT-X-FOO:A="),
        Err(Error::InvalidAttributeList {
            reason: AttributeListError::EmptyValue,
            ..
        })
    ));

    assert!(AttrValue::quoted("a\tb").is_ok());
    assert!(AttrValue::quoted("a\rb").is_err());
    assert!(AttrValue::bare("a\u{7f}b").is_ok());
    assert!(matches!(
        tag.set_attr("C", AttrValue::quoted("a\tb").expect("general value")),
        Err(Error::InvalidAttributeList {
            reason: AttributeListError::ControlCharacter,
            ..
        })
    ));
    assert!(matches!(
        DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",X-A=\"a\tb\""),
        Err(Error::InvalidAttributeList {
            reason: AttributeListError::ControlCharacter,
            ..
        })
    ));
    assert!(matches!(
        CustomTag::parse_tag_line("#EXT-X-FOO:A=\"a\tb\""),
        Err(Error::InvalidAttributeList {
            reason: AttributeListError::ControlCharacter,
            ..
        })
    ));
    assert!(
        AttrValue::quoted("a\u{80}b").is_ok(),
        "general path keeps it"
    );
    assert!(matches!(
        DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",X-A=\"a\u{80}b\""),
        Err(Error::InvalidAttributeList {
            reason: AttributeListError::ControlCharacter,
            ..
        })
    ));
    assert!(matches!(
        CustomTag::parse_tag_line("#EXT-X-FOO:A=\"a\u{80}b\""),
        Err(Error::InvalidAttributeList {
            reason: AttributeListError::ControlCharacter,
            ..
        })
    ));
    assert!(matches!(
        DateRange::new("a\u{80}b"),
        Err(Error::InvalidAttributeList {
            reason: AttributeListError::ControlCharacter,
            ..
        })
    ));
    let mut range = DateRange::new("a").expect("valid id");
    assert!(matches!(
        range.set_class(Some("a\u{9f}b")),
        Err(Error::InvalidAttributeList {
            reason: AttributeListError::ControlCharacter,
            ..
        })
    ));
}

/// Round 5 item 1: whole-playlist parsing keeps origin/main's tolerance of a
/// tab in an unknown quoted attribute.
#[test]
fn whole_playlist_parsing_keeps_control_tolerance() {
    use broadcast_hls::{MasterPlaylist, MediaPlaylist};

    let media = MediaPlaylist::parse(
        "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MAP:URI=\"i.mp4\",X-FOO=\"a\tb\"\n#EXTINF:2,\ns.ts\n",
    )
    .expect("whole-playlist parse must keep its historical tolerance");
    let map = media.segments[0].map.as_ref().expect("map");
    let value = &map
        .extra_attrs
        .iter()
        .find(|(n, _)| n == "X-FOO")
        .expect("X-FOO")
        .1;
    assert_eq!(value.as_str(), "a\tb");

    let master =
        MasterPlaylist::parse("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,X-FOO=\"a\tb\"\nv.m3u8\n")
            .expect("whole-playlist parse must keep its historical tolerance");
    assert_eq!(master.variants[0].extra_attrs[0].0, "X-FOO");
    assert_eq!(master.variants[0].extra_attrs[0].1.as_str(), "a\tb");
}

/// Round 5 item 3: the even-digit rule is SCTE35-only; an `X-` hexadecimal
/// sequence may have an odd digit count.
#[test]
fn hex_length_rule_is_scte35_only() {
    use broadcast_hls::{DateRangeError, Scte35Kind};

    daterange_roundtrip("#EXT-X-DATERANGE:ID=\"a\",X-A=0xABC");
    assert!(matches!(
        DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",SCTE35-OUT=0xABC"),
        Err(Error::InvalidDateRange {
            reason: DateRangeError::InvalidScte35Value,
            ..
        })
    ));
    let mut range = DateRange::new("a").expect("valid id");
    assert!(matches!(
        range.set_scte35(Scte35Kind::Out, "0xABC"),
        Err(Error::InvalidDateRange {
            reason: DateRangeError::InvalidScte35Value,
            ..
        })
    ));
    range
        .set_scte35(Scte35Kind::Out, "0xABCD")
        .expect("whole bytes");
}

/// Item 4 (round 4): an `SCTE35-*` attribute is built through a validated,
/// typed setter.
#[test]
fn set_scte35_builds_a_valid_splice_attribute() {
    use broadcast_hls::{DateRangeError, Scte35Kind};

    let mut range = DateRange::new("a").expect("valid id");
    range
        .set_scte35(Scte35Kind::Out, "0xFCFF")
        .expect("valid hex");
    assert_eq!(
        range.to_tag_line().expect("render"),
        "#EXT-X-DATERANGE:ID=\"a\",SCTE35-OUT=0xFCFF"
    );
    let parsed = DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",SCTE35-OUT=0xFCFF")
        .expect("must parse");
    assert_eq!(parsed, range);
    assert!(matches!(
        range.set_scte35(Scte35Kind::Cmd, "FCFF"),
        Err(Error::InvalidDateRange {
            reason: DateRangeError::InvalidScte35Value,
            ..
        })
    ));
    assert_eq!(Scte35Kind::In.name(), "SCTE35-IN");
    assert_eq!(Scte35Kind::In.to_string(), "SCTE35-IN");
}

/// Item 2: an empty quoted-string is legal for a quoted-string attribute,
/// while an empty `ID` stays a typed error.
#[test]
fn empty_quoted_values_are_accepted_except_the_id() {
    use broadcast_hls::{CustomTag, DateRangeError};

    daterange_roundtrip("#EXT-X-DATERANGE:ID=\"a\",X-A=\"\"");
    daterange_roundtrip("#EXT-X-DATERANGE:ID=\"a\",CLASS=\"\"");
    let tag = CustomTag::parse_tag_line("#EXT-X-FOO:A=\"\"").expect("empty quoted value");
    assert_eq!(tag.to_tag_line().expect("render"), "#EXT-X-FOO:A=\"\"");
    assert!(matches!(
        DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"\""),
        Err(Error::InvalidDateRange {
            reason: DateRangeError::EmptyId,
            ..
        })
    ));
}

/// RFC 8216bis §4.4.5.1: an `X-` value may be a signed-decimal-floating-point,
/// so a negative decimal parses, round-trips byte-identically and is accepted
/// by the setter; a lone `-` is still not a decimal.
#[test]
fn client_decimal_values_may_be_signed() {
    use broadcast_hls::{AttrValue, ClientAttrName, DateRangeError};

    daterange_roundtrip("#EXT-X-DATERANGE:ID=\"a\",X-A=1.5");
    daterange_roundtrip("#EXT-X-DATERANGE:ID=\"a\",X-A=-1.5");
    let mut range = DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\"").expect("parse");
    range
        .set_client_attr(
            ClientAttrName::new("X-B").expect("name"),
            AttrValue::bare("-2").expect("bare"),
        )
        .expect("a signed decimal is a legal X- value");
    assert_eq!(
        range.to_tag_line().expect("render"),
        "#EXT-X-DATERANGE:ID=\"a\",X-B=-2"
    );
    assert!(matches!(
        DateRange::parse_tag_line("#EXT-X-DATERANGE:ID=\"a\",X-A=-"),
        Err(Error::InvalidDateRange {
            reason: DateRangeError::InvalidClientAttributeValue,
            ..
        }) | Err(Error::InvalidAttributeList { .. })
    ));
}

/// Cosmetic: an error whose `ID` is later in the line still names that id.
#[test]
fn daterange_errors_report_the_id_when_it_is_known() {
    use broadcast_hls::DateRangeError;

    assert!(matches!(
        DateRange::parse_tag_line("#EXT-X-DATERANGE:CUE=\"PRE,POST\",ID=\"real\""),
        Err(Error::InvalidDateRange {
            id,
            reason: DateRangeError::CuePreAndPost,
        }) if id == "real"
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

/// Item 4: without pinning `version`, appending one fractional-duration
/// segment changes the header, so a writer must pin it.
#[test]
fn unpinned_version_changes_an_already_written_header() {
    use broadcast_hls::{MediaPlaylist, MediaSegment};

    let mut playlist = MediaPlaylist {
        target_duration: 3,
        segments: vec![MediaSegment {
            uri: "seg0.ts".into(),
            duration: broadcast_hls::DecimalSeconds::new(2.0).expect("valid"),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(
        playlist.render_header().expect("header"),
        "#EXTM3U\n#EXT-X-TARGETDURATION:3\n#EXT-X-MEDIA-SEQUENCE:0\n"
    );
    playlist.segments.push(MediaSegment {
        uri: "seg1.ts".into(),
        duration: broadcast_hls::DecimalSeconds::new(2.5).expect("valid"),
        ..Default::default()
    });
    assert_eq!(
        playlist.render_header().expect("header"),
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:3\n#EXT-X-MEDIA-SEQUENCE:0\n",
        "the fractional segment triggers §8 row 3 after the header was written"
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

/// Item 6: a pending `#EXTINF`/`#EXT-X-BYTERANGE` overwritten before its URI
/// line is reported too, not just one left dangling at EOF.
#[test]
fn overwritten_segment_tags_are_each_reported() {
    use broadcast_hls::{MediaPlaylist, ParseDiagnosticKind};

    let input = "#EXTM3U\n#EXT-X-TARGETDURATION:3\n#EXTINF:2,\n#EXTINF:3,\nb.ts\n";
    let playlist = MediaPlaylist::parse(input).expect("must parse");
    assert_eq!(playlist.segments.len(), 1);
    assert_eq!(playlist.segments[0].duration.get(), 3.0);
    assert_eq!(playlist.diagnostics.len(), 1, "{:?}", playlist.diagnostics);
    assert_eq!(
        playlist.diagnostics[0].kind,
        ParseDiagnosticKind::DanglingSegmentTags
    );
    assert_eq!(playlist.diagnostics[0].line, "#EXTINF:2,");

    let input = "#EXTM3U\n#EXT-X-TARGETDURATION:3\n#EXTINF:3,\n#EXT-X-BYTERANGE:10@0\n#EXT-X-BYTERANGE:20@10\nb.ts\n";
    let playlist = MediaPlaylist::parse(input).expect("must parse");
    assert_eq!(playlist.segments.len(), 1);
    assert_eq!(
        playlist.segments[0].byte_range.expect("byte range").length,
        20
    );
    assert_eq!(playlist.diagnostics.len(), 1, "{:?}", playlist.diagnostics);
    assert_eq!(
        playlist.diagnostics[0].kind,
        ParseDiagnosticKind::DanglingSegmentTags
    );
    assert_eq!(playlist.diagnostics[0].line, "#EXT-X-BYTERANGE:10@0");
}

/// RFC 8216bis §4.4.5.1: `START-DATE` is REQUIRED unless an earlier
/// `EXT-X-DATERANGE` with the same `ID` in the same playlist carries one. A
/// single tag cannot see that earlier tag, so the playlist parser reports the
/// violation; the tag itself is kept and re-renders unchanged.
#[test]
fn daterange_without_start_date_is_a_playlist_diagnostic() {
    use broadcast_hls::{MediaPlaylist, ParseDiagnosticKind};

    let augmenting = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n\
        #EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:00Z\n\
        #EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2026-01-01T00:00:00Z\"\n\
        #EXTINF:2,\ns0.ts\n\
        #EXT-X-DATERANGE:ID=\"a\",DURATION=4\n\
        #EXTINF:2,\ns1.ts\n";
    let playlist = MediaPlaylist::parse(augmenting).expect("must parse");
    assert!(playlist.diagnostics.is_empty(), "{:?}", playlist.diagnostics);

    let orphan = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n\
        #EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:00Z\n\
        #EXT-X-DATERANGE:ID=\"b\",DURATION=4\n\
        #EXTINF:2,\ns0.ts\n\
        #EXT-X-DATERANGE:ID=\"b\",START-DATE=\"2026-01-01T00:00:00Z\"\n\
        #EXTINF:2,\ns1.ts\n";
    let playlist = MediaPlaylist::parse(orphan).expect("must parse");
    assert_eq!(playlist.diagnostics.len(), 1, "{:?}", playlist.diagnostics);
    let diag = &playlist.diagnostics[0];
    assert_eq!(diag.kind, ParseDiagnosticKind::DateRangeWithoutStartDate);
    assert_eq!(diag.line_no, 4);
    assert_eq!(diag.line, "#EXT-X-DATERANGE:ID=\"b\",DURATION=4");
    assert_eq!(
        ParseDiagnosticKind::DateRangeWithoutStartDate.to_string(),
        "date_range_without_start_date"
    );
    assert!(
        playlist.segments[0]
            .pre_tags
            .iter()
            .any(|t| t == "#EXT-X-DATERANGE:ID=\"b\",DURATION=4"),
        "the reported tag is kept: {:?}",
        playlist.segments[0].pre_tags
    );
}
