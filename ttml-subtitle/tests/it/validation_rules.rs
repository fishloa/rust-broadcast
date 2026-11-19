//! IMSC 1.1 validator rule coverage (#3078): each rule gets one conforming
//! and one violating document, asserting the reported constraint.
//!
//! Rules: IMSC 1.1 §7.9, §7.12.7, §7.12.1.3, §8.4.11, §9.4.1; time-expression
//! well-formedness is TTML2 §12.3.1.
#![cfg(feature = "std")]

use ttml_subtitle::document::{IMSC11_TEXT_PROFILE, IMSC1_TEXT_PROFILE, NS_TT, NS_TTP, NS_TTS};
use ttml_subtitle::validation::{ImscVersion, Profile, ValidationResult, Validator};
use ttml_subtitle::Document;

fn validate(xml: &str, profile: Profile, version: ImscVersion) -> ValidationResult {
    let doc = Document::parse_str(xml).unwrap_or_else(|e| panic!("parse: {e}\n{xml}"));
    Validator::new(profile, version).validate(&doc)
}

fn has(result: &ValidationResult, constraint: &str) -> bool {
    result.errors.iter().any(|e| e.constraint == constraint)
}

fn doc_with_profile(profile: &str, params: &str, inner: &str) -> String {
    format!(
        r#"<tt xmlns="{NS_TT}" xmlns:ttp="{NS_TTP}" xmlns:tts="{NS_TTS}" ttp:profile="{profile}" {params}>{inner}</tt>"#
    )
}

const EMPTY_BODY: &str = "<body/>";

#[test]
fn text_profile_claim_is_required_for_text_validation() {
    let xml = doc_with_profile(IMSC1_TEXT_PROFILE, "", EMPTY_BODY);
    let text = validate(&xml, Profile::Text, ImscVersion::V1_0);
    assert!(text.valid, "{:?}", text.errors);
    let image = validate(&xml, Profile::Image, ImscVersion::V1_0);
    assert!(has(&image, "IMSC §7.9"), "{:?}", image.errors);
}

#[test]
fn image_profile_claim_is_required_for_image_validation() {
    let xml = doc_with_profile(ttml_subtitle::document::IMSC1_IMAGE_PROFILE, "", EMPTY_BODY);
    let image = validate(&xml, Profile::Image, ImscVersion::V1_0);
    assert!(image.valid, "{:?}", image.errors);
    let text = validate(&xml, Profile::Text, ImscVersion::V1_0);
    assert!(has(&text, "IMSC §7.9"), "{:?}", text.errors);
}

#[test]
fn content_profiles_claim_is_recognised() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:ttp="{NS_TTP}" ttp:contentProfiles="{IMSC1_TEXT_PROFILE}"><body/></tt>"#
    );
    let text = validate(&xml, Profile::Text, ImscVersion::V1_0);
    assert!(text.valid, "{:?}", text.errors);
    let image = validate(&xml, Profile::Image, ImscVersion::V1_0);
    assert!(has(&image, "IMSC §7.9"), "{:?}", image.errors);
}

#[test]
fn nested_unknown_timing_only_checks_tt_div_p_image() {
    let inner = r#"<body><div>
        <x:div begin="garbage"/>
        <div note="garbage"/>
        <foo begin="garbage"/>
      </div></body>"#;
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:ttp="{NS_TTP}" xmlns:x="urn:x" ttp:profile="{IMSC1_TEXT_PROFILE}">{inner}</tt>"#
    );
    let result = validate(&xml, Profile::Text, ImscVersion::V1_0);
    assert!(result.valid, "{:?}", result.errors);
}

#[test]
fn nested_unknown_div_timing_is_checked() {
    let inner = r#"<body><div><div begin="garbage"/></div></body>"#;
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:ttp="{NS_TTP}" ttp:profile="{IMSC1_TEXT_PROFILE}">{inner}</tt>"#
    );
    let result = validate(&xml, Profile::Text, ImscVersion::V1_0);
    assert!(has(&result, "IMSC §12.3.1"), "{:?}", result.errors);
}

fn frame_usage_cases() -> Vec<String> {
    let mut cases = Vec::new();
    for attr in ["begin", "dur", "end"] {
        cases.push(format!(r#"<body {attr}="10f"><div><p>x</p></div></body>"#));
        cases.push(format!(
            r#"<body><div {attr}="10f"><p>x</p></div></body>"#
        ));
        cases.push(format!(
            r#"<body><div><p {attr}="10f">x</p></div></body>"#
        ));
        cases.push(format!(
            r#"<body><div><image src="a.png" {attr}="10f"/></div></body>"#
        ));
    }
    cases
}

#[test]
fn frame_usage_without_frame_rate_is_an_error_at_every_position() {
    for inner in frame_usage_cases() {
        let xml = doc_with_profile(IMSC11_TEXT_PROFILE, "", &inner);
        let result = validate(&xml, Profile::Text, ImscVersion::V1_1);
        assert!(
            has(&result, "IMSC §7.12.7"),
            "missing §7.12.7 for {inner}: {:?}",
            result.errors
        );
    }
}

#[test]
fn frame_usage_with_frame_rate_is_accepted() {
    for inner in frame_usage_cases() {
        let xml = doc_with_profile(IMSC11_TEXT_PROFILE, r#"ttp:frameRate="30""#, &inner);
        let result = validate(&xml, Profile::Text, ImscVersion::V1_1);
        assert!(
            !has(&result, "IMSC §7.12.7"),
            "unexpected §7.12.7 for {inner}: {:?}",
            result.errors
        );
    }
}

#[test]
fn single_character_f_expression_is_not_frame_usage() {
    let xml = doc_with_profile(
        IMSC11_TEXT_PROFILE,
        "",
        r#"<body><div><p begin="f">x</p></div></body>"#,
    );
    let result = validate(&xml, Profile::Text, ImscVersion::V1_1);
    assert!(
        !has(&result, "IMSC §7.12.7"),
        "unexpected §7.12.7: {:?}",
        result.errors
    );
    assert!(has(&result, "IMSC §12.3.1"), "{:?}", result.errors);
}

#[test]
fn clock_time_without_frames_is_not_frame_usage() {
    let xml = doc_with_profile(
        IMSC11_TEXT_PROFILE,
        "",
        r#"<body begin="00:00:01"><div><p>x</p></div></body>"#,
    );
    let result = validate(&xml, Profile::Text, ImscVersion::V1_1);
    assert!(
        !has(&result, "IMSC §7.12.7"),
        "unexpected §7.12.7: {:?}",
        result.errors
    );
}

#[test]
fn clock_time_with_frames_is_frame_usage() {
    let xml = doc_with_profile(
        IMSC11_TEXT_PROFILE,
        "",
        r#"<body begin="00:00:01:05"><div><p>x</p></div></body>"#,
    );
    let result = validate(&xml, Profile::Text, ImscVersion::V1_1);
    assert!(has(&result, "IMSC §7.12.7"), "{:?}", result.errors);
}

fn layout_with(regions: usize) -> String {
    let body: String = (0..regions)
        .map(|i| format!(r#"<region xml:id="r{i}"/>"#))
        .collect();
    doc_with_profile(
        IMSC1_TEXT_PROFILE,
        "",
        &format!("<head><layout>{body}</layout></head><body/>"),
    )
}

#[test]
fn at_most_four_presented_regions() {
    let four = validate(&layout_with(4), Profile::Text, ImscVersion::V1_0);
    assert!(four.valid, "{:?}", four.errors);
    let five = validate(&layout_with(5), Profile::Text, ImscVersion::V1_0);
    assert!(has(&five, "IMSC §7.12.1.3"), "{:?}", five.errors);
}

#[test]
fn text_profile_allows_paragraphs() {
    let xml = doc_with_profile(
        IMSC1_TEXT_PROFILE,
        "",
        r#"<body><div><p begin="0s" end="1s">x</p></div></body>"#,
    );
    let result = validate(&xml, Profile::Text, ImscVersion::V1_0);
    assert!(result.valid, "{:?}", result.errors);
}

#[test]
fn image_profile_rejects_paragraphs() {
    let xml = doc_with_profile(
        ttml_subtitle::document::IMSC1_IMAGE_PROFILE,
        "",
        r#"<body><div><p>x</p></div></body>"#,
    );
    let result = validate(&xml, Profile::Image, ImscVersion::V1_0);
    assert!(has(&result, "IMSC §9.4.1"), "{:?}", result.errors);
}

fn text_shadow_doc(shadows: &str) -> String {
    doc_with_profile(
        IMSC1_TEXT_PROFILE,
        "",
        &format!(
            r#"<body><div><p begin="0s" end="1s" tts:textShadow="{shadows}">x</p></div></body>"#
        ),
    )
}

#[test]
fn text_shadow_allows_four_values_and_rejects_five() {
    const FOUR: &str = "1px 1px 1px red,1px 1px 1px red,1px 1px 1px red,1px 1px 1px red";
    const FIVE: &str =
        "1px 1px 1px red,1px 1px 1px red,1px 1px 1px red,1px 1px 1px red,1px 1px 1px red";

    let four = validate(&text_shadow_doc(FOUR), Profile::Text, ImscVersion::V1_0);
    assert!(four.valid, "{:?}", four.errors);
    let five = validate(&text_shadow_doc(FIVE), Profile::Text, ImscVersion::V1_0);
    assert!(has(&five, "IMSC §8.4.11"), "{:?}", five.errors);
}
