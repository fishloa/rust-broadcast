//! Fixture tests for `check_dash_mpd` — DASH MPD validation using
//! structured model (transmux::Mpd::parse).
//!
//! Fixtures live in `fixtures/dash/`:
//! - `manifest.mpd` — well-formed static MPD → zero findings.

use std::fs;

use media_doctor::{Report, check_dash_mpd};

fn read_fixture(name: &str) -> String {
    let path = format!("{}/../fixtures/dash/{}", env!("CARGO_MANIFEST_DIR"), name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
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
