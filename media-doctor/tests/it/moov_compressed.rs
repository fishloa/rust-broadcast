use media_doctor::{Report, check_container_codec};

#[test]
fn compressed_moov_fixture_reports_no_codec_findings() {
    let mut bytes = test_fixtures::read("mp4/h264-aac-prog.mp4");
    let pos = bytes.windows(4).position(|w| w == b"moov").expect("moov");
    bytes[pos..pos + 4].copy_from_slice(b"!mov");
    let mut report = Report::new();
    check_container_codec(&bytes, &mut report);
    let codec_findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| {
            f.rule_id.contains("codec") || f.rule_id.contains("avcc") || f.rule_id.contains("hvcc")
        })
        .collect();
    assert!(codec_findings.is_empty(), "{codec_findings:?}");
}
