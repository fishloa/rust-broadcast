//! Fixture tests for `check_dash_mpd` — DASH MPD validation using
//! structured model (broadcast_dash::Mpd::parse).
//!
//! Fixtures live in `fixtures/dash/`:
//! - `manifest.mpd` — well-formed static MPD → zero findings.
#![cfg(feature = "std")]

use media_doctor::{Report, check_dash_mpd};

fn read_fixture(name: &str) -> String {
    test_fixtures::read_to_string(&format!("dash/{name}"))
}

fn findings<'a>(report: &'a Report, rule_id: &str) -> Vec<&'a media_doctor::Finding> {
    report
        .findings()
        .iter()
        .filter(|f| f.rule_id == rule_id)
        .collect()
}

// -------------------------------------------------------------------------
// Clean manifest — zero findings
// -------------------------------------------------------------------------

/// The committed `manifest.mpd` is a real ffmpeg-generated DASH MPD with
/// two AdaptationSets, SegmentTimeline addressing — must produce zero findings.
#[test]
fn valid_mpd_clean() {
    let text = read_fixture("manifest.mpd");
    let mut report = Report::new();
    check_dash_mpd(&text, &mut report);

    assert!(
        report.is_empty(),
        "valid MPD should produce no findings, got {}: {:?}",
        report.len(),
        report.findings(),
    );
}

// -------------------------------------------------------------------------
// Negative tests — violations are detected
// -------------------------------------------------------------------------

/// Malformed XML → `dash-parse-error`.
#[test]
fn malformed_xml_detected() {
    // Not even XML
    let text = "this is not XML";
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);

    let hits = findings(&report, "dash-parse-error");
    assert!(
        !hits.is_empty(),
        "malformed input should produce dash-parse-error, got {:?}",
        report.findings(),
    );
}

/// Static MPD without `mediaPresentationDuration` → `dash-static-mpd-missing-duration`.
#[test]
fn static_mpd_missing_duration_detected() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static">
  <Period id="0">
    <AdaptationSet id="0" contentType="video">
      <Representation id="0" bandwidth="500000" codecs="avc1.4d400d" width="320" height="240">
        <SegmentTemplate timescale="90000" initialization="init$RepresentationID$.m4s" media="chunk$RepresentationID$-$Number%05d$.m4s" startNumber="1" duration="90000"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);

    let hits = findings(&report, "dash-static-mpd-missing-duration");
    assert!(
        !hits.is_empty(),
        "static MPD missing duration should produce finding, got {:?}",
        report.findings(),
    );
}

/// Duplicate Representation @id → `dash-representation-id-duplicate`.
#[test]
fn duplicate_rep_id_detected() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT1M">
  <Period id="0">
    <AdaptationSet id="0" contentType="video">
      <Representation id="0" bandwidth="500000" codecs="avc1.4d400d" width="320" height="240">
        <SegmentTemplate timescale="90000" initialization="init$RepresentationID$.m4s" media="chunk$RepresentationID$-$Number%05d$.m4s" startNumber="1" duration="90000"/>
      </Representation>
      <Representation id="0" bandwidth="1000000" codecs="avc1.4d401e" width="640" height="480">
        <SegmentTemplate timescale="90000" initialization="init$RepresentationID$.m4s" media="chunk$RepresentationID$-$Number%05d$.m4s" startNumber="1" duration="90000"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);

    let hits = findings(&report, "dash-representation-id-duplicate");
    assert!(
        !hits.is_empty(),
        "duplicate @id should produce finding, got {:?}",
        report.findings(),
    );
}

/// SegmentTimeline with non-monotonic t values → `dash-segment-timeline-monotonic`.
#[test]
fn non_monotonic_timeline_detected() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT5S">
  <Period id="0">
    <AdaptationSet id="0" contentType="video">
      <Representation id="0" bandwidth="500000" codecs="avc1.4d400d" width="320" height="240">
        <SegmentTemplate timescale="90000" initialization="init$RepresentationID$.m4s" media="chunk$RepresentationID$-$Time$.m4s" startNumber="1">
          <SegmentTimeline>
            <S t="0" d="90000" r="1"/>
            <S t="45000" d="90000"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);

    let hits = findings(&report, "dash-segment-timeline-monotonic");
    assert!(
        !hits.is_empty(),
        "non-monotonic timeline should produce finding, got {:?}",
        report.findings(),
    );
}

/// Empty Period (no AdaptationSets) → `dash-period-no-adaptation-sets`.
#[test]
fn empty_period_detected() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT1M">
  <Period id="empty" />
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);

    let hits = findings(&report, "dash-period-no-adaptation-sets");
    assert!(
        !hits.is_empty(),
        "empty period should produce finding, got {:?}",
        report.findings(),
    );
}

/// First `<S>` without `@t` is legal — §5.3.9.6.2 says it defaults to 0.
/// The validator must NOT panic; it must produce zero findings.
#[test]
fn first_s_without_t_is_clean() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT5S">
  <Period id="0">
    <AdaptationSet id="0" contentType="video">
      <Representation id="0" bandwidth="500000" codecs="avc1.4d400d" width="320" height="240">
        <SegmentTemplate timescale="90000" initialization="init$RepresentationID$.m4s" media="chunk$RepresentationID$-$Time$.m4s" startNumber="1">
          <SegmentTimeline>
            <S d="90000" r="2"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);

    assert!(
        report.is_empty(),
        "first S without @t should be clean (defaults to 0 per §5.3.9.6.2), got {}: {:?}",
        report.len(),
        report.findings(),
    );
}

// -------------------------------------------------------------------------
// Issue #1112, audit MD-W12: @id uniqueness is per Period
// -------------------------------------------------------------------------

/// ISO/IEC 23009-1 §5.3.5.2 Table 7: `Representation@id` "shall be unique
/// within a Period". The realistic violation is the *same* `@id` in two
/// different AdaptationSets of one Period — checking per AdaptationSet, as
/// this did before, could never see it.
#[test]
fn duplicate_representation_id_across_adaptation_sets_is_flagged() {
    let text = r#"<?xml version="1.0" encoding="utf-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
     mediaPresentationDuration="PT10S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <Representation id="dup" bandwidth="1000000" width="640" height="360">
        <BaseURL>v1.mp4</BaseURL>
      </Representation>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" contentType="audio">
      <Representation id="dup" bandwidth="128000">
        <BaseURL>a1.mp4</BaseURL>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>
"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);

    let hits = findings(&report, "dash-representation-id-duplicate");
    assert_eq!(
        hits.len(),
        1,
        "the same @id in two AdaptationSets of one Period must be flagged; got {:?}",
        report.findings(),
    );
    assert!(
        hits[0].message.contains("dup") && hits[0].message.contains("Period"),
        "the finding must name the id and the Period scope, got {:?}",
        hits[0].message,
    );
}

/// Two Representations of different AdaptationSets with *different* `@id`s
/// are fine — the rule is uniqueness, not one-id-per-set.
#[test]
fn distinct_representation_ids_are_clean() {
    let text = r#"<?xml version="1.0" encoding="utf-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
     mediaPresentationDuration="PT10S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <Representation id="v1" bandwidth="1000000" width="640" height="360">
        <BaseURL>v1.mp4</BaseURL>
      </Representation>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" contentType="audio">
      <Representation id="a1" bandwidth="128000">
        <BaseURL>a1.mp4</BaseURL>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>
"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);
    assert!(
        findings(&report, "dash-representation-id-duplicate").is_empty(),
        "distinct @ids across AdaptationSets are legal; got {:?}",
        report.findings(),
    );
}

/// `@r = -1` is legal and means "repeat until the next `S` element or the end
/// of the Period" (ISO/IEC 23009-1 §5.3.9.6.2 Table 17). It must not be
/// treated as a single repeat, and the following `S` element must not be
/// reported as non-monotonic against a guessed end.
#[test]
fn segment_timeline_negative_r_until_next_s_is_not_flagged() {
    let text = r#"<?xml version="1.0" encoding="utf-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
     mediaPresentationDuration="PT10S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <SegmentTemplate timescale="90000" initialization="init.mp4" media="seg-$Number$.m4s" startNumber="1">
        <SegmentTimeline>
          <S t="0" d="180000" r="-1"/>
          <S t="90000" d="180000"/>
        </SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v1" bandwidth="1000000" width="640" height="360"/>
    </AdaptationSet>
  </Period>
</MPD>
"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);
    assert!(
        findings(&report, "dash-segment-timeline-monotonic").is_empty(),
        "@r = -1 repeats until the next S, whose explicit @t governs; got {:?}",
        report.findings(),
    );
}

/// `@id` duplicated by two **functionally identical** Representations is
/// redundant, not ambiguous: ISO/IEC 23009-1 §5.3.5.2 makes `@id` unique
/// within a Period *unless* the Representations are identical, so this is a
/// Warning.
#[test]
fn duplicate_id_of_identical_representations_is_a_warning() {
    let text = r#"<?xml version="1.0" encoding="utf-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
     mediaPresentationDuration="PT10S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <Representation id="same" bandwidth="1000000" codecs="avc1.64001f" width="640" height="360">
        <BaseURL>v1.mp4</BaseURL>
      </Representation>
      <Representation id="same" bandwidth="1000000" codecs="avc1.64001f" width="640" height="360">
        <BaseURL>v1.mp4</BaseURL>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>
"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);
    let hits = findings(&report, "dash-representation-id-duplicate");
    assert_eq!(hits.len(), 1, "got {:?}", report.findings());
    assert_eq!(
        hits[0].severity,
        media_doctor::Severity::Warning,
        "identical Representations are redundant, not ambiguous: {:?}",
        hits[0],
    );
}

/// `@r` below -1 is out of range: Table 17 defines only `@r >= -1`.
#[test]
fn segment_timeline_r_below_minus_one_is_an_error() {
    let text = r#"<?xml version="1.0" encoding="utf-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
     mediaPresentationDuration="PT10S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <SegmentTemplate timescale="90000" initialization="init.mp4" media="seg-$Number$.m4s" startNumber="1">
        <SegmentTimeline>
          <S t="0" d="180000" r="-5"/>
        </SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v1" bandwidth="1000000" width="640" height="360"/>
    </AdaptationSet>
  </Period>
</MPD>
"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);
    let hits = findings(&report, "dash-segment-timeline-r-range");
    assert_eq!(
        hits.len(),
        1,
        "@r = -5 is out of range and must be reported; got {:?}",
        report.findings(),
    );
    assert_eq!(hits[0].severity, media_doctor::Severity::Error);
}

#[test]
fn static_mpd_with_last_period_duration_has_no_missing_duration_error() {
    let text = test_fixtures::read_to_string("dash/manifest.mpd");
    let text = text
        .replace("mediaPresentationDuration=\"PT3.0S\"", "")
        .replace(
            "<Period id=\"0\" start=\"PT0.0S\">",
            "<Period id=\"0\" start=\"PT0.0S\" duration=\"PT3.0S\">",
        );
    assert!(
        text.contains("duration=\"PT3.0S\">"),
        "fixture edit applied"
    );
    assert!(!text.contains("mediaPresentationDuration"));
    let mut report = Report::new();
    check_dash_mpd(&text, &mut report);
    let hits: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "dash-static-mpd-missing-duration")
        .collect();
    assert!(
        hits.is_empty(),
        "23009-1 Table 3: @mediaPresentationDuration is required only when neither @minimumUpdatePeriod nor the last Period@duration is present; got {hits:?}"
    );
}

// -------------------------------------------------------------------------
// Parse bounds, element locations and duplicate-@id determinism (audit 23)
// -------------------------------------------------------------------------

/// The validator exists to vaildate a manifest, so it reads it with every
/// bound relaxed: more Periods than `broadcast_dash::limits::MAX_PERIODS`
/// must be validated, not reported as an unparseable document.
#[test]
fn a_manifest_beyond_the_default_period_cap_is_still_validated() {
    let periods = broadcast_dash::limits::MAX_PERIODS + 1;

    let mut text = String::from(
        "<?xml version=\"1.0\"?>\n\
         <MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" \
         mediaPresentationDuration=\"PT1001S\">\n",
    );
    for i in 0..periods {
        text.push_str(&format!(
            "<Period id=\"p{i}\"><AdaptationSet mimeType=\"video/mp4\" contentType=\"video\">\
             <Representation id=\"v\" bandwidth=\"1000\" width=\"320\" height=\"240\">\
             <BaseURL>v.mp4</BaseURL></Representation></AdaptationSet></Period>\n",
        ));
    }
    text.push_str("</MPD>\n");

    let mut report = Report::new();
    check_dash_mpd(&text, &mut report);
    assert!(
        findings(&report, "dash-parse-error").is_empty(),
        "the relaxed limits must read this manifest: {:?}",
        report.findings(),
    );
    assert!(
        report.is_empty(),
        "{periods} Periods is one more than the default bound and is still a \
         valid manifest; got {} finding(s): {:?}",
        report.len(),
        report.findings(),
    );
}

/// `Period` findings carry the 1-based Period position, not an unrelated
/// number. The empty Period is second, so a `*` or a constant cannot pass.
#[test]
fn empty_period_finding_names_the_second_period() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <Representation id="v" bandwidth="1000" width="320" height="240"><BaseURL>v.mp4</BaseURL></Representation>
    </AdaptationSet>
  </Period>
  <Period id="p1" />
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);
    let hits = findings(&report, "dash-period-no-adaptation-sets");
    assert_eq!(hits.len(), 1, "{:?}", report.findings());
    assert_eq!(
        hits[0].location.packet, 2,
        "the second Period is position 2"
    );
}

/// A monotonic `<S>` finding names the 1-based position of the offending
/// element: the third `<S>` here starts before the second one ends.
#[test]
fn non_monotonic_timeline_finding_names_the_third_s() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <Representation id="v" bandwidth="1000">
        <SegmentTemplate timescale="90000" initialization="i.mp4" media="s-$Time$.m4s">
          <SegmentTimeline>
            <S t="0" d="90000"/>
            <S t="90000" d="90000"/>
            <S t="100000" d="90000"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);
    let hits = findings(&report, "dash-segment-timeline-monotonic");
    assert_eq!(hits.len(), 1, "{:?}", report.findings());
    assert_eq!(hits[0].location.packet, 3, "the third <S> is position 3");
}

/// An out-of-range `@r` finding names the 1-based position of its `<S>`:
/// `@r = -2` sits on the second element here.
#[test]
fn out_of_range_r_finding_names_the_second_s() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <Representation id="v" bandwidth="1000">
        <SegmentTemplate timescale="90000" initialization="i.mp4" media="s-$Time$.m4s">
          <SegmentTimeline>
            <S t="0" d="90000"/>
            <S t="90000" d="90000" r="-2"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);
    let hits = findings(&report, "dash-segment-timeline-r-range");
    assert_eq!(hits.len(), 1, "{:?}", report.findings());
    assert_eq!(hits[0].location.packet, 2, "the second <S> is position 2");
}

/// The duplicate-`@id` severity is decided by whole-Representation identity:
/// ISO/IEC 23009-1 §5.3.5.2 makes the duplicate redundant only when the two
/// Representations are functionally identical. Every field is compared, and a
/// difference in any one of them keeps it an Error.
fn duplicate_id_severity(second: &str) -> media_doctor::Severity {
    let text = format!(
        "<?xml version=\"1.0\"?>\n\
         <MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" \
         mediaPresentationDuration=\"PT10S\">\n\
         <Period id=\"p0\"><AdaptationSet mimeType=\"video/mp4\" contentType=\"video\">\n\
         <Representation id=\"dup\" bandwidth=\"1000000\" codecs=\"avc1.64001f\" \
         width=\"640\" height=\"360\"><BaseURL>v1.mp4</BaseURL></Representation>\n\
         {second}\n\
         </AdaptationSet></Period></MPD>\n",
    );
    let mut report = Report::new();
    check_dash_mpd(&text, &mut report);
    let hits = findings(&report, "dash-representation-id-duplicate");
    assert_eq!(hits.len(), 1, "{second:?} -> {:?}", report.findings());
    hits[0].severity
}

fn representation(attributes: &str) -> String {
    format!("<Representation id=\"dup\" {attributes}><BaseURL>v1.mp4</BaseURL></Representation>")
}

/// Identical in every compared field — redundant, so a Warning.
#[test]
fn duplicate_id_of_identical_representations_without_template_is_a_warning() {
    let second =
        representation("bandwidth=\"1000000\" codecs=\"avc1.64001f\" width=\"640\" height=\"360\"");
    assert_eq!(
        duplicate_id_severity(&second),
        media_doctor::Severity::Warning,
    );
}

/// Differing only in bandwidth is still two different Representations.
#[test]
fn duplicate_id_differing_only_in_bandwidth_is_an_error() {
    let second =
        representation("bandwidth=\"2000000\" codecs=\"avc1.64001f\" width=\"640\" height=\"360\"");
    assert_eq!(
        duplicate_id_severity(&second),
        media_doctor::Severity::Error,
    );
}

/// Differing only in codecs is still two different Representations.
#[test]
fn duplicate_id_differing_only_in_codecs_is_an_error() {
    let second =
        representation("bandwidth=\"1000000\" codecs=\"avc1.640028\" width=\"640\" height=\"360\"");
    assert_eq!(
        duplicate_id_severity(&second),
        media_doctor::Severity::Error,
    );
}

/// Differing only in width is still two different Representations.
#[test]
fn duplicate_id_differing_only_in_width_is_an_error() {
    let second = representation(
        "bandwidth=\"1000000\" codecs=\"avc1.64001f\" width=\"1280\" height=\"360\"",
    );
    assert_eq!(
        duplicate_id_severity(&second),
        media_doctor::Severity::Error,
    );
}

/// Same codec/geometry, different `SegmentTemplate@media`: the two address
/// different segments, so the duplicate is ambiguous, not redundant.
#[test]
fn duplicate_id_differing_only_in_segment_template_is_an_error() {
    let template = |media: &str| {
        format!(
            "<Representation id=\"dup\" bandwidth=\"1000000\" codecs=\"avc1.64001f\" \
             width=\"640\" height=\"360\"><BaseURL>v1.mp4</BaseURL>\
             <SegmentTemplate timescale=\"90000\" initialization=\"i.mp4\" \
             media=\"{media}\"/></Representation>"
        )
    };
    let text = format!(
        "<?xml version=\"1.0\"?>\n\
         <MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" \
         mediaPresentationDuration=\"PT10S\">\n\
         <Period id=\"p0\"><AdaptationSet mimeType=\"video/mp4\" contentType=\"video\">\n\
         {}\n{}\n\
         </AdaptationSet></Period></MPD>\n",
        template("a-$Number$.m4s"),
        template("b-$Number$.m4s"),
    );
    let mut report = Report::new();
    check_dash_mpd(&text, &mut report);
    let hits = findings(&report, "dash-representation-id-duplicate");
    assert_eq!(hits.len(), 1, "{:?}", report.findings());
    assert_eq!(hits[0].severity, media_doctor::Severity::Error);
}

/// With the same `SegmentTemplate` as well, nothing distinguishes them.
#[test]
fn duplicate_id_of_identical_representations_with_template_is_a_warning() {
    let template = "<Representation id=\"dup\" bandwidth=\"1000000\" codecs=\"avc1.64001f\" \
         width=\"640\" height=\"360\"><BaseURL>v1.mp4</BaseURL>\
         <SegmentTemplate timescale=\"90000\" initialization=\"i.mp4\" \
         media=\"a-$Number$.m4s\"/></Representation>";
    let text = format!(
        "<?xml version=\"1.0\"?>\n\
         <MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" \
         mediaPresentationDuration=\"PT10S\">\n\
         <Period id=\"p0\"><AdaptationSet mimeType=\"video/mp4\" contentType=\"video\">\n\
         {template}\n{template}\n\
         </AdaptationSet></Period></MPD>\n",
    );
    let mut report = Report::new();
    check_dash_mpd(&text, &mut report);
    let hits = findings(&report, "dash-representation-id-duplicate");
    assert_eq!(hits.len(), 1, "{:?}", report.findings());
    assert_eq!(hits[0].severity, media_doctor::Severity::Warning);
}

/// The monotonic-timeline message prints both bounds in seconds at a
/// non-unit timescale (90000 ticks = 1.000 s).
#[test]
fn monotonic_timeline_message_prints_seconds_at_a_non_unit_timescale() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <Representation id="v" bandwidth="1000">
        <SegmentTemplate timescale="90000" initialization="i.mp4" media="s-$Time$.m4s">
          <SegmentTimeline>
            <S t="0" d="90000"/>
            <S t="45000" d="90000"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);
    let hits = findings(&report, "dash-segment-timeline-monotonic");
    assert_eq!(hits.len(), 1, "{:?}", report.findings());
    assert!(
        hits[0].message.contains("t=45000 (0.500s)"),
        "{:?}",
        hits[0].message
    );
    assert!(
        hits[0].message.contains("prev_end=90000 (1.000s)"),
        "{:?}",
        hits[0].message
    );
}

/// At a timescale of 1 a tick is a second already: the message prints the
/// bare count and no seconds suffix.
#[test]
fn monotonic_timeline_message_prints_bare_ticks_at_unit_timescale() {
    let text = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S">
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <Representation id="v" bandwidth="1000">
        <SegmentTemplate timescale="1" initialization="i.mp4" media="s-$Time$.m4s">
          <SegmentTimeline>
            <S t="0" d="10"/>
            <S t="5" d="10"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
    let mut report = Report::new();
    check_dash_mpd(text, &mut report);
    let hits = findings(&report, "dash-segment-timeline-monotonic");
    assert_eq!(hits.len(), 1, "{:?}", report.findings());
    assert!(hits[0].message.contains("t=5>"), "{:?}", hits[0].message);
    assert!(
        !hits[0].message.contains("s)"),
        "a unit timescale prints ticks, not seconds: {:?}",
        hits[0].message
    );
}
