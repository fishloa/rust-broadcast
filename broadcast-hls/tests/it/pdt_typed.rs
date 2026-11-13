use broadcast_hls::{
    DecimalSeconds, MediaPlaylist, MediaSegment, ParseDiagnosticKind, ProgramDateTime,
};

#[test]
fn parse_spec_example_offset() {
    let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-PROGRAM-DATE-TIME:2010-02-19T14:54:23.031+08:00\n\
#EXTINF:6.000,\n\
s0.m4s\n\
#EXT-X-ENDLIST\n";
    let pl = MediaPlaylist::parse(text).expect("parse");
    let pdt = pl.segments[0].program_date_time.expect("pdt");
    assert_eq!(pdt.to_string(), "2010-02-19T06:54:23.031Z");
    // canonical render
    let out = pl.to_m3u8().unwrap();
    assert!(out.contains("#EXT-X-PROGRAM-DATE-TIME:2010-02-19T06:54:23.031Z\n"));
}

#[test]
fn no_zone_treated_as_utc() {
    let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:00\n\
#EXTINF:6.000,\n\
s0.m4s\n";
    let pl = MediaPlaylist::parse(text).expect("parse");
    let pdt = pl.segments[0].program_date_time.expect("pdt");
    assert_eq!(pdt.to_string(), "2024-01-01T00:00:00.000Z");
}

/// A bad `EXT-X-PROGRAM-DATE-TIME` no longer fails the playlist (the owner's decision on #2591):
/// the tag is dropped, its segment has no program date time, and the diagnostic names the line.
#[test]
fn an_invalid_value_is_dropped_and_reported_with_its_line() {
    let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-PROGRAM-DATE-TIME:not-a-date\n\
#EXTINF:6.000,\n\
s0.m4s\n";
    let pl = MediaPlaylist::parse(text).expect("the playlist still parses");
    assert_eq!(pl.segments.len(), 1);
    assert_eq!(pl.segments[0].program_date_time, None);
    assert_eq!(pl.diagnostics.len(), 1, "{:?}", pl.diagnostics);
    let diagnostic = &pl.diagnostics[0];
    assert_eq!(diagnostic.kind, ParseDiagnosticKind::InvalidProgramDateTime);
    assert_eq!(diagnostic.line_no, 4);
    assert_eq!(diagnostic.line, "#EXT-X-PROGRAM-DATE-TIME:not-a-date");
    assert_eq!(diagnostic.kind.to_string(), "invalid_program_date_time");
    assert!(!pl.to_m3u8().unwrap().contains("PROGRAM-DATE-TIME"));
}

/// A bare date is not a date-time (RFC 8216 §4.3.2.6): `ProgramDateTime::parse` rejects it, so the
/// playlist parser drops the tag and reports it, while the neighbouring valid tag still parses.
#[test]
fn a_bare_date_is_dropped_and_a_valid_neighbour_still_parses() {
    let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-PROGRAM-DATE-TIME:2026-10-02\n\
#EXTINF:6.000,\n\
s0.m4s\n\
#EXT-X-PROGRAM-DATE-TIME:2026-10-02T10:00:06Z\n\
#EXTINF:6.000,\n\
s1.m4s\n";
    let pl = MediaPlaylist::parse(text).expect("parse");
    assert_eq!(pl.segments[0].program_date_time, None);
    assert_eq!(
        pl.segments[1]
            .program_date_time
            .expect("valid pdt")
            .to_string(),
        "2026-10-02T10:00:06.000Z"
    );
    assert_eq!(pl.diagnostics.len(), 1);
    assert_eq!(pl.diagnostics[0].line_no, 4);
    assert_eq!(
        pl.diagnostics[0].line,
        "#EXT-X-PROGRAM-DATE-TIME:2026-10-02"
    );
}

#[test]
fn a_valid_value_leaves_no_diagnostic() {
    let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-PROGRAM-DATE-TIME:2026-10-02T10:00:00Z\n\
#EXTINF:6.000,\n\
s0.m4s\n";
    let pl = MediaPlaylist::parse(text).expect("parse");
    assert!(pl.diagnostics.is_empty());
    assert!(pl.segments[0].program_date_time.is_some());
}

#[test]
fn serialize_parse_equality() {
    let pdt = ProgramDateTime::parse("2024-01-01T00:00:00.000Z").unwrap();
    let pl = MediaPlaylist {
        target_duration: 6,
        segments: vec![MediaSegment {
            uri: "s0.m4s".into(),
            duration: DecimalSeconds::new(6.0).unwrap(),
            program_date_time: Some(pdt),
            ..Default::default()
        }],
        ..Default::default()
    };
    let out = pl.to_m3u8().unwrap();
    let parsed = MediaPlaylist::parse(&out).expect("reparse");
    assert_eq!(parsed, pl);
}

fn pos(out: &str, needle: &str) -> usize {
    out.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} missing in:\n{out}"))
}

#[test]
fn closed_segment_order_is_discontinuity_map_pdt_parts_extinf() {
    // §4.4.4.9: DISCONTINUITY/KEY/MAP/PROGRAM-DATE-TIME applied to a Parent
    // Segment MUST appear before its first EXT-X-PART.
    let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXT-X-MAP:URI=\"init.mp4\"\n\
#EXT-X-DISCONTINUITY\n\
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:00.000Z\n\
#EXT-X-PART:DURATION=0.5,URI=\"p0.m4s\"\n\
#EXTINF:6.000,\n\
s0.m4s\n";
    let pl = MediaPlaylist::parse(text).expect("parse");
    let out = pl.to_m3u8().unwrap();
    let d = pos(&out, "#EXT-X-DISCONTINUITY");
    let m = pos(&out, "#EXT-X-MAP:");
    let t = pos(&out, "#EXT-X-PROGRAM-DATE-TIME:");
    let p = pos(&out, "#EXT-X-PART:");
    let e = pos(&out, "#EXTINF:");
    assert!(d < m && m < t && t < p && p < e, "order wrong:\n{out}");
    let parsed = MediaPlaylist::parse(&out).expect("reparse");
    assert_eq!(parsed.segments, pl.segments);
}

#[test]
fn pdt_on_open_segment_precedes_its_first_part() {
    let text = "#EXTM3U\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-PART-INF:PART-TARGET=1.0\n\
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:00.000Z\n\
#EXT-X-PART:DURATION=1.0,URI=\"p0.m4s\"\n";
    let pl = MediaPlaylist::parse(text).expect("parse");
    let open = pl.open_segment.as_ref().expect("open");
    assert!(open.program_date_time.is_some());
    let out = pl.to_m3u8().unwrap();
    assert!(
        pos(&out, "#EXT-X-PROGRAM-DATE-TIME:") < pos(&out, "#EXT-X-PART:"),
        "PDT must precede the open segment's first part:\n{out}"
    );
    let reparsed = MediaPlaylist::parse(&out).expect("reparse");
    assert_eq!(reparsed.open_segment, pl.open_segment);
}

#[test]
fn from_timestamp_truncates_to_milliseconds() {
    let ts: jiff::Timestamp = "2024-01-01T00:00:00.123456789Z".parse().unwrap();
    let pdt = ProgramDateTime::from_timestamp(ts);
    assert_eq!(pdt.to_string(), "2024-01-01T00:00:00.123Z");
    assert_eq!(pdt.timestamp().as_millisecond(), ts.as_millisecond());
}

#[test]
fn canonicalisation_offset_and_sub_ms() {
    // offset case: textual form canonicalised to UTC
    let a = ProgramDateTime::parse("2010-02-19T14:54:23.031+08:00").unwrap();
    let b = ProgramDateTime::parse("2010-02-19T06:54:23.031Z").unwrap();
    assert_eq!(a, b);
    assert_eq!(a.to_string(), "2010-02-19T06:54:23.031Z");
    // sub-ms precision truncated: typed values equal after round trip
    let c = ProgramDateTime::parse("2024-01-01T00:00:00.123456Z").unwrap();
    let rendered = c.to_string();
    assert_eq!(rendered, "2024-01-01T00:00:00.123Z");
    let d = ProgramDateTime::parse(&rendered).unwrap();
    assert_eq!(c, d);
}
