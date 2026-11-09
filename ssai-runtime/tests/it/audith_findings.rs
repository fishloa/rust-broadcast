//! Bug audit H reproductions: each test fails on main.
use ssai_runtime::{AssetSource, InterstitialDateRange};

fn dr() -> InterstitialDateRange {
    InterstitialDateRange {
        id: "a".into(),
        start_date: "2020-01-02T21:55:44.000Z".into(),
        duration: Some(15.0),
        asset: AssetSource::Uri("http://x/a.m3u8".into()),
        resume_offset: None,
        playout_limit: None,
        snap: vec![],
        restrict: vec![],
    }
}

#[test]
fn h_s1_negative_x_resume_offset_is_rejected_on_render_and_parse() {
    let mut d = dr();
    d.resume_offset = Some(-5.0);
    let line = d
        .to_tag_line()
        .expect("X-RESUME-OFFSET is signed-decimal-floating-point");
    let back = InterstitialDateRange::parse_tag_line(&line).unwrap();
    assert_eq!(back.resume_offset, Some(-5.0));
}

#[test]
fn h_s1b_parse_negative_x_resume_offset() {
    let s = "#EXT-X-DATERANGE:ID=\"a\",CLASS=\"com.apple.hls.interstitial\",START-DATE=\"2020-01-02T21:55:44.000Z\",X-ASSET-URI=\"http://x/a\",X-RESUME-OFFSET=-3";
    assert!(InterstitialDateRange::parse_tag_line(s).is_ok());
}

#[test]
fn h_s2_duration_2_pow_63_renders_off_by_one() {
    let mut d = dr();
    d.duration = Some(9223372036854775808.0);
    let l = d.to_tag_line().unwrap();
    assert!(l.contains("DURATION=9223372036854775808"), "{l}");
}
