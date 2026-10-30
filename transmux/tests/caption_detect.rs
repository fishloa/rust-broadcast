//! Caption presence detection over real captures (issue #1428).

use broadcast_common::Unpackage;
use transmux::{CaptionDetector, CodecConfig, DetectedCaption, TsDemux};

fn detect(fixture: &str, test: &str) -> Option<Vec<DetectedCaption>> {
    let data = test_fixtures::require_bytes(&format!("ts/{fixture}.ts"), test)?;
    let media = TsDemux::new()
        .unpackage(data.as_slice())
        .expect("demux fixture");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("fixture has an H.264 track");
    let mut detector = CaptionDetector::for_config(video.config()).expect("AVC has a detector");
    for sample in &video.samples {
        detector.observe(&sample.data);
    }
    Some(detector.detected())
}

#[test]
fn detects_cea608_field1_in_a53_sei_of_real_fixture() {
    let Some(found) = detect(
        "transformers-eia608-h264",
        "detects_cea608_field1_in_a53_sei_of_real_fixture",
    ) else {
        return;
    };
    assert_eq!(
        found,
        vec![
            DetectedCaption::Cea608Field1,
            DetectedCaption::Cea708Service(1)
        ]
    );
}

#[test]
fn no_captions_detected_in_fixture_without_cc() {
    let Some(found) = detect("h264_aac", "no_captions_detected_in_fixture_without_cc") else {
        return;
    };
    assert!(found.is_empty(), "unexpected captions: {found:?}");
}
