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
use dvb_si::descriptors::atsc_caption_service::{AtscCaptionKind, AtscCaptionService};
use dvb_si::text::LangCode;
use transmux::{CaptionDetector, CodecConfig, DetectedCaption, TsDemux, declared_caption_services};

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

const ATSC_DECLARED: &str = "tsduck/input/test-040.ts";

fn atsc_video_descriptors(test: &str) -> Option<Vec<u8>> {
    let data = test_fixtures::require_bytes(ATSC_DECLARED, test)?;
    let media = TsDemux::new()
        .unpackage(data.as_slice())
        .expect("demux fixture");
    let video = media
        .tracks
        .iter()
        .find(|t| t.spec.source_pid.is_some_and(|p| p.value() == 0x61))
        .expect("PID 0x61 video track");
    Some(video.spec.es_info_descriptors.clone())
}

#[test]
fn reads_caption_service_descriptor_from_real_pmt() {
    let Some(descriptors) =
        atsc_video_descriptors("reads_caption_service_descriptor_from_real_pmt")
    else {
        return;
    };
    let services = declared_caption_services(&descriptors);
    let shape: Vec<_> = services
        .iter()
        .map(|s| (s.language.as_str().to_string(), s.kind))
        .collect();
    assert_eq!(
        shape,
        vec![
            (
                "eng".to_string(),
                AtscCaptionKind::Cea608 {
                    reserved: 0b11111,
                    line21_field: false
                }
            ),
            (
                "eng".to_string(),
                AtscCaptionKind::Cea608 {
                    reserved: 0b11111,
                    line21_field: true
                }
            ),
            (
                "eng".to_string(),
                AtscCaptionKind::Cea708 {
                    caption_service_number: 1
                }
            ),
        ]
    );
}

#[test]
fn declared_language_matches_708_by_service_number_and_608_when_unambiguous() {
    let Some(descriptors) = atsc_video_descriptors(
        "declared_language_matches_708_by_service_number_and_608_when_unambiguous",
    ) else {
        return;
    };
    let services = declared_caption_services(&descriptors);
    assert_eq!(
        DetectedCaption::Cea708Service(1).declared_language(&services),
        Some("eng".to_string())
    );
    assert_eq!(
        DetectedCaption::Cea708Service(2).declared_language(&services),
        None
    );
    assert_eq!(
        DetectedCaption::Cea608(Cea608Channel::Cc3).declared_language(&services),
        Some("eng".to_string())
    );
}

#[test]
fn conflicting_608_languages_are_not_guessed() {
    let services = vec![
        AtscCaptionService::cea608(LangCode(*b"eng"), false, false, true),
        AtscCaptionService::cea608(LangCode(*b"spa"), true, false, true),
        AtscCaptionService::cea708(LangCode(*b"fre"), 2, false, true),
    ];
    assert_eq!(
        DetectedCaption::Cea608(Cea608Channel::Cc1).declared_language(&services),
        None
    );
    assert_eq!(
        DetectedCaption::Cea708Service(2).declared_language(&services),
        Some("fre".to_string())
    );
    assert_eq!(
        DetectedCaption::Cea708Service(1).declared_language(&services),
        None
    );
}

#[test]
fn no_descriptor_declares_nothing() {
    assert!(declared_caption_services(&[]).is_empty());
    assert!(declared_caption_services(&[0x0A, 0x04, b'e', b'n', b'g', 0x00]).is_empty());
}
