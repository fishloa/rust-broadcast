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
