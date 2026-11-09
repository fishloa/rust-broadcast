#![cfg(feature = "std")]

use media_doctor::{Report, check_dash_mpd};

#[test]
fn static_mpd_with_last_period_duration_has_no_missing_duration_error() {
    let text = test_fixtures::read_to_string("dash/manifest.mpd");
    let text = text
        .replace("mediaPresentationDuration=\"PT3.0S\"", "")
        .replace(
            "<Period id=\"0\" start=\"PT0.0S\">",
            "<Period id=\"0\" start=\"PT0.0S\" duration=\"PT3.0S\">",
        );
    assert!(text.contains("duration=\"PT3.0S\">"), "fixture edit applied");
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

#[test]
fn avcc_level_above_the_sps_level_is_conformant() {
    let mut bytes = std::fs::read(test_fixtures::path("mp4/h264-aac-prog.mp4")).unwrap();
    let mut baseline = Report::new();
    media_doctor::check_container_codec(&bytes, &mut baseline);
    assert!(
        baseline.findings().iter().all(|f| f.rule_id != "avcc-sps-mismatch"),
        "precondition: the unmodified fixture is clean: {:?}",
        baseline.findings()
    );
    let at = bytes.windows(4).position(|w| w == b"avcC").expect("avcC box");
    let level = at + 4 + 3;
    bytes[level] = bytes[level].saturating_add(1);
    let mut report = Report::new();
    media_doctor::check_container_codec(&bytes, &mut report);
    let hits: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "avcc-sps-mismatch")
        .collect();
    assert!(
        hits.is_empty(),
        "14496-15:2017 5.3.3.1: AVCLevelIndication need only be >= the highest SPS level; got {hits:?}"
    );
}
