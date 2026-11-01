//! Caption presence detection over real captures (issue #1428).
//!
//! The expected sets were derived independently of the detector by a Python
//! walk over the per-SEI `cc_data()` reference lists
//! (`private/fixtures/captions/*.cc_data.txt`, written by
//! `fixtures/captions/gen_cc_data_reference.py`): per field, the channel bit of
//! the last control code routes each text pair, XDS is skipped, and DTVCC
//! packets are split into service blocks.

use broadcast_common::Unpackage;
use cc_data::decode::Cea608Channel;
use transmux::{CaptionDetector, CodecConfig, DetectedCaption, TsDemux};

fn detect(fixture: &str, test: &str) -> Option<Vec<DetectedCaption>> {
    let data = test_fixtures::require_bytes(fixture, test)?;
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
fn detects_cea608_cc1_and_service_1_in_a53_sei_of_real_fixture() {
    let Some(found) = detect(
        "ts/transformers-eia608-h264.ts",
        "detects_cea608_cc1_and_service_1_in_a53_sei_of_real_fixture",
    ) else {
        return;
    };
    assert_eq!(
        found,
        vec![
            DetectedCaption::Cea608(Cea608Channel::Cc1),
            DetectedCaption::Cea708Service(1)
        ]
    );
}

#[test]
fn detects_one_channel_per_field_in_two_field_capture() {
    let Some(found) = detect(
        "captions/mux.js/multi-channel-608-captions.ts",
        "detects_one_channel_per_field_in_two_field_capture",
    ) else {
        return;
    };
    assert_eq!(
        found,
        vec![
            DetectedCaption::Cea608(Cea608Channel::Cc1),
            DetectedCaption::Cea608(Cea608Channel::Cc3)
        ]
    );
}

#[test]
fn detects_cc1_and_dtvcc_service_1_in_shaka_capture() {
    let Some(found) = detect(
        "captions/shaka/captions-test.ts",
        "detects_cc1_and_dtvcc_service_1_in_shaka_capture",
    ) else {
        return;
    };
    assert_eq!(
        found,
        vec![
            DetectedCaption::Cea608(Cea608Channel::Cc1),
            DetectedCaption::Cea708Service(1)
        ]
    );
}

#[test]
fn no_captions_detected_in_fixture_without_cc() {
    let Some(found) = detect(
        "ts/h264_aac.ts",
        "no_captions_detected_in_fixture_without_cc",
    ) else {
        return;
    };
    assert!(found.is_empty(), "unexpected captions: {found:?}");
}
