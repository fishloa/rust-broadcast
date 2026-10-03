//! Integration tests: parse all 11 real IMSC fixtures, round-trip,
//! profile validation, time expression exhaustive tests.
#![cfg(feature = "std")]

use std::fs;
use std::path::PathBuf;

use ttml_subtitle::document::{BodyElement, DivElement, InlineContent, PElement, SpanElement};
use ttml_subtitle::*;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
}

// ─── Fixture round-trip tests ─────────────────────────────────────

/// Helper: normalize whitespace in a document for semantic comparison.
/// Whitespace-only Text nodes are stripped; non-empty text is trimmed of
/// leading/trailing whitespace. This makes semantic equality possible across
/// XML round-trips where indentation/whitespace cannot be preserved.
fn normalize_doc(doc: &mut Document) {
    // Root-level inter-element text is no longer captured on TtElement
    // (it was whitespace noise; #1110/TT-W1 removed the dead `text` field).
    // Normalize body content
    if let Some(ref mut body) = doc.tt.body {
        normalize_body(body);
    }
}

fn normalize_body(body: &mut BodyElement) {
    for div in &mut body.divs {
        normalize_div(div);
    }
}

fn normalize_div(div: &mut DivElement) {
    for p in &mut div.paragraphs {
        normalize_p(p);
    }
}

fn normalize_p(p: &mut PElement) {
    // Strip leading/trailing whitespace-only Text nodes
    let mut content: Vec<InlineContent> = Vec::new();
    for item in p.content.drain(..) {
        match item {
            InlineContent::Text(text) => {
                // Keep non-empty text; strip leading whitespace if preceded by other content
                let trimmed = text.trim();
                let has_non_ws = text.chars().any(|c| !c.is_whitespace());
                if has_non_ws {
                    content.push(InlineContent::Text(trimmed.to_string()));
                }
            }
            InlineContent::Span(mut span) => {
                normalize_span(&mut span);
                content.push(InlineContent::Span(span));
            }
            other => content.push(other),
        }
    }
    p.content = content;
}

fn normalize_span(span: &mut SpanElement) {
    let mut content: Vec<InlineContent> = Vec::new();
    for item in span.content.drain(..) {
        match item {
            InlineContent::Text(text) => {
                let trimmed = text.trim();
                let has_non_ws = text.chars().any(|c| !c.is_whitespace());
                if has_non_ws {
                    content.push(InlineContent::Text(trimmed.to_string()));
                }
            }
            InlineContent::Span(mut child_span) => {
                normalize_span(&mut child_span);
                content.push(InlineContent::Span(child_span));
            }
            other => content.push(other),
        }
    }
    span.content = content;
}

/// Helper: parse → serialize → re-parse → normalize → assert semantic equality.
fn round_trip(fixture_name: &str) {
    let xml = load_fixture(fixture_name);
    let mut doc = Document::parse_str(&xml)
        .unwrap_or_else(|e| panic!("failed to parse fixture {fixture_name}: {e}"));
    normalize_doc(&mut doc);

    // Re-serialize
    let regenerated = doc.to_xml();

    // Re-parse
    let mut doc2 = Document::parse_str(&regenerated).unwrap_or_else(|e| {
        panic!(
            "failed to re-parse regenerated XML for {fixture_name}: {e}\nRegenerated:\n{regenerated}"
        )
    });
    normalize_doc(&mut doc2);

    // Assert semantic equality
    assert_eq!(
        doc, doc2,
        "round-trip semantic equality failed for {fixture_name}"
    );
}

/// Helper: parse → mutate a field → serialize → re-parse → assert
/// the field change is reflected. This is a BITING round-trip test:
/// a raw-passthrough serializer would not update the output.
fn biting_round_trip(fixture_name: &str) {
    let xml = load_fixture(fixture_name);
    let mut doc = Document::parse_str(&xml).unwrap();
    normalize_doc(&mut doc);

    // Mutate: change the first p's begin attribute if it exists
    let mut mutated = false;
    if let Some(ref mut body) = doc.tt.body {
        for div in &mut body.divs {
            for p in &mut div.paragraphs {
                if p.begin.is_some() {
                    // Change begin time
                    p.begin = Some("99s".to_string());
                    mutated = true;
                    break;
                }
            }
            if mutated {
                break;
            }
        }
    }

    if !mutated {
        // If no p has begin, try adding a style attribute to first paragraph
        if let Some(ref mut body) = doc.tt.body {
            for div in &mut body.divs {
                if let Some(p) = div.paragraphs.iter_mut().next() {
                    p.style_attributes.tts_color = Some("red".to_string());
                    mutated = true;
                    break;
                }
                if mutated {
                    break;
                }
            }
        }
    }

    if !mutated {
        // Document has no mutable children; skip biting test
        return;
    }

    let regenerated = doc.to_xml();
    let mut doc2 = Document::parse_str(&regenerated).unwrap();
    normalize_doc(&mut doc2);

    // Assert the mutation is reflected (after normalization)
    assert_eq!(doc, doc2, "biting round-trip: mutated field not preserved");

    // Also assert the regenerated XML contains the mutation
    assert!(
        regenerated.contains("99s") || regenerated.contains("red"),
        "biting round-trip: mutated document doesn't contain expected mutation in output"
    );
}

macro_rules! fixture_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            round_trip($file);
        }
    };
}

macro_rules! fixture_biting_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            biting_round_trip($file);
        }
    };
}

fixture_test!(
    round_trip_document_example,
    "imsc1-document-example-822.ttml"
);
fixture_test!(
    round_trip_time_expressions,
    "imsc1-time-expressions-001.ttml"
);
fixture_test!(round_trip_animation, "imsc1-animation-001.ttml");
fixture_test!(
    round_trip_backgroundcolor_rgba,
    "imsc1-backgroundcolor-rgba-001.ttml"
);
fixture_test!(round_trip_ruby, "imsc1.1-ruby-001.ttml");
fixture_test!(round_trip_textemphasis, "imsc1.1-textemphasis-001.ttml");
fixture_test!(round_trip_textshadow, "imsc1.1-textshadow-001.ttml");
fixture_test!(
    round_trip_displayaspectratio,
    "imsc1.1-displayaspectratio-001.ttml"
);
fixture_test!(round_trip_activearea, "imsc1-activearea-001.ttml");
fixture_test!(
    round_trip_alttext_smpte,
    "imsc1-alttext-smpte-backgroundimage-001.ttml"
);
fixture_test!(round_trip_image_profile, "imsc1.1-image-profile-001.ttml");

fixture_biting_test!(biting_document_example, "imsc1-document-example-822.ttml");
fixture_biting_test!(biting_time_expressions, "imsc1-time-expressions-001.ttml");
fixture_biting_test!(biting_animation, "imsc1-animation-001.ttml");

// ─── Profile validation tests ─────────────────────────────────────

#[test]
fn validate_text_profile_document() {
    let xml = load_fixture("imsc1-document-example-822.ttml");
    let doc = Document::parse_str(&xml).unwrap();

    let validator =
        validation::Validator::new(validation::Profile::Text, validation::ImscVersion::V1_0);
    let result = validator.validate(&doc);
    assert!(
        result.valid,
        "Text Profile document should validate as Text: {:?}",
        result.errors
    );
}

#[test]
fn validate_text_profile_1_1() {
    let xml = load_fixture("imsc1.1-ruby-001.ttml");
    let doc = Document::parse_str(&xml).unwrap();

    let validator =
        validation::Validator::new(validation::Profile::Text, validation::ImscVersion::V1_1);
    let result = validator.validate(&doc);
    assert!(
        result.valid,
        "IMSC 1.1 Text Profile document should validate: {:?}",
        result.errors
    );
}

#[test]
fn validate_image_profile_document() {
    let xml = load_fixture("imsc1.1-image-profile-001.ttml");
    let doc = Document::parse_str(&xml).unwrap();

    let validator =
        validation::Validator::new(validation::Profile::Image, validation::ImscVersion::V1_1);
    let result = validator.validate(&doc);
    assert!(
        result.valid,
        "Image Profile document should validate: {:?}",
        result.errors
    );
}

#[test]
fn reject_5_plus_regions() {
    // Hand-constructed document with 5 regions (violates §7.12.1.3)
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xml:lang="en" xmlns="http://www.w3.org/ns/ttml"
   xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
   xmlns:tts="http://www.w3.org/ns/ttml#styling"
   ttp:contentProfiles="http://www.w3.org/ns/ttml/profile/imsc1.1/text">
  <head>
    <layout>
      <region xml:id="r1" tts:extent="10% 10%" tts:origin="0% 0%"/>
      <region xml:id="r2" tts:extent="10% 10%" tts:origin="20% 0%"/>
      <region xml:id="r3" tts:extent="10% 10%" tts:origin="40% 0%"/>
      <region xml:id="r4" tts:extent="10% 10%" tts:origin="60% 0%"/>
      <region xml:id="r5" tts:extent="10% 10%" tts:origin="80% 0%"/>
    </layout>
  </head>
  <body>
    <div>
      <p region="r1" begin="0s" end="1s">R1</p>
      <p region="r2" begin="0s" end="1s">R2</p>
      <p region="r3" begin="0s" end="1s">R3</p>
      <p region="r4" begin="0s" end="1s">R4</p>
      <p region="r5" begin="0s" end="1s">R5</p>
    </div>
  </body>
</tt>"#;

    let doc = Document::parse_str(xml).unwrap();
    let validator =
        validation::Validator::new(validation::Profile::Text, validation::ImscVersion::V1_1);
    let result = validator.validate(&doc);
    assert!(!result.valid, "Document with 5 regions should be rejected");
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.constraint.contains("7.12.1.3")),
        "Should cite §7.12.1.3 constraint, got: {:?}",
        result.errors
    );
}

#[test]
fn reject_image_profile_with_text_content() {
    // Hand-constructed: Image Profile document with a <p> element (§9.4.1)
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xml:lang="en" xmlns="http://www.w3.org/ns/ttml"
   xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
   xmlns:tts="http://www.w3.org/ns/ttml#styling"
   ttp:contentProfiles="http://www.w3.org/ns/ttml/profile/imsc1.1/image"
   tts:extent="640px 480px"
   ttp:displayAspectRatio="4 3">
  <head>
    <layout>
      <region xml:id="r1" tts:extent="640px 480px" tts:origin="0px 0px"/>
    </layout>
  </head>
  <body>
    <div region="r1" begin="0s" end="5s">
      <p begin="0s" end="5s">This should not be allowed in Image Profile</p>
      <image tts:extent="640px 480px" src="test.png" type="image/png"/>
    </div>
  </body>
</tt>"#;

    let doc = Document::parse_str(xml).unwrap();
    let validator =
        validation::Validator::new(validation::Profile::Image, validation::ImscVersion::V1_1);
    let result = validator.validate(&doc);
    assert!(!result.valid, "Image Profile with <p> should be rejected");
    assert!(
        result.errors.iter().any(|e| e.constraint.contains("9.4.1")),
        "Should cite §9.4.1 constraint, got: {:?}",
        result.errors
    );
}

// TT-W3 (#1108): the validator implements only a handful of checks — these
// two are real bugs the audit found in what IS implemented, not "claims"
// fixes.

#[test]
fn validator_rejects_malformed_time_expression() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" ttp:contentProfiles="http://www.w3.org/ns/ttml/profile/imsc1.1/text"
    xmlns:ttp="http://www.w3.org/ns/ttml#parameter">
  <body>
    <div>
      <p begin="garbage" end="1s">hi</p>
    </div>
  </body>
</tt>"#;
    let doc = Document::parse_str(xml).unwrap();
    let validator =
        validation::Validator::new(validation::Profile::Text, validation::ImscVersion::V1_1);
    let result = validator.validate(&doc);
    // Pre-fix, `begin="garbage"` was never run through
    // `time::parse_time_expression`, so this validated as `true`.
    assert!(!result.valid, "malformed begin= must be rejected");
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.constraint.contains("12.3.1")),
        "should cite §12.3.1, got: {:?}",
        result.errors
    );
}

#[test]
fn validator_checks_frame_usage_on_body_with_no_div() {
    // `<body>` itself carries a frame-term begin, but has no `<div>` at all.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" ttp:contentProfiles="http://www.w3.org/ns/ttml/profile/imsc1.1/text"
    xmlns:ttp="http://www.w3.org/ns/ttml#parameter">
  <body begin="10f"/>
</tt>"#;
    let doc = Document::parse_str(xml).unwrap();
    let validator =
        validation::Validator::new(validation::Profile::Text, validation::ImscVersion::V1_1);
    let result = validator.validate(&doc);
    // Pre-fix, the check for `body`'s own begin/dur/end lived inside `for
    // div in &body.divs`, so it never ran when there were zero divs, and
    // this document (frame term used, no ttp:frameRate) validated as `true`.
    assert!(
        !result.valid,
        "frame term on body with no div must still require ttp:frameRate"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.constraint.contains("7.12.7")),
        "should cite §7.12.7, got: {:?}",
        result.errors
    );
}

// ─── Time expression exhaustive tests ─────────────────────────────

#[test]
fn time_expression_exhaustive_fixture_form() {
    // Test each time expression in the fixture to ensure round-trip
    let xml = load_fixture("imsc1-time-expressions-001.ttml");
    let doc = Document::parse_str(&xml).unwrap();

    // Collect all time expressions from the document
    let mut expressions: Vec<String> = Vec::new();
    if let Some(ref body) = doc.tt.body {
        for div in &body.divs {
            for p in &div.paragraphs {
                if let Some(ref b) = p.begin {
                    expressions.push(b.clone());
                }
                if let Some(ref e) = p.end {
                    expressions.push(e.clone());
                }
            }
        }
    }

    let ctx = doc.tt.time_context();
    for expr in &expressions {
        let parsed = time::parse_time_expression(expr, &ctx)
            .unwrap_or_else(|e| panic!("failed to parse time expr '{expr}': {e}"));
        let formatted = time::format_time_expression(&parsed);
        let re_parsed = time::parse_time_expression(&formatted, &ctx)
            .unwrap_or_else(|e| panic!("failed to re-parse '{formatted}': {e}"));
        assert_eq!(
            parsed, re_parsed,
            "time expression round-trip failed: '{expr}' → '{formatted}'"
        );
    }
}

// TT-W4 (#1108): default tickRate (§7.2.11).

#[test]
fn tick_rate_defaults_to_1_when_no_frame_rate_specified() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml">
  <body><div><p begin="0s" end="1s">hi</p></div></body>
</tt>"#;
    let doc = Document::parse_str(xml).unwrap();
    let ctx = doc.tt.time_context();
    // Pre-fix this was 30 (frame_rate's OWN default) * 1 (sub_frame_rate's
    // own default) = 30, even though no `ttp:frameRate` was ever specified.
    assert_eq!(ctx.tick_rate, 1);
}

#[test]
fn tick_rate_uses_effective_frame_rate_with_multiplier() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
    ttp:frameRate="30" ttp:frameRateMultiplier="1000 1001" ttp:subFrameRate="2">
  <body><div><p begin="0s" end="1s">hi</p></div></body>
</tt>"#;
    let doc = Document::parse_str(xml).unwrap();
    let ctx = doc.tt.time_context();
    // effective frame rate = 30 * 1000 / 1001 = 29 (integer division);
    // tick_rate = 29 * 2 = 58. Pre-fix, the multiplier was parsed but never
    // used here, so this came out as 30 * 2 = 60 instead.
    assert_eq!(ctx.tick_rate, 58);
}

#[test]
fn tick_rate_explicit_value_still_wins() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
    ttp:tickRate="10000">
  <body><div><p begin="0s" end="1s">hi</p></div></body>
</tt>"#;
    let doc = Document::parse_str(xml).unwrap();
    let ctx = doc.tt.time_context();
    assert_eq!(ctx.tick_rate, 10_000);
}

#[test]
fn time_expression_malformed_rejected() {
    let ctx = time::TimeContext::default();

    let bad_exprs = vec![
        "",
        "abc",
        "12:34",                          // missing seconds
        "00:60:00",                       // minutes out of range
        "00:00:61",                       // seconds out of range
        "0:00:00",                        // hours < 10 without leading zero
        "00:0:00",                        // minutes < 10 without leading zero
        "00:00:0",                        // seconds < 10 without leading zero
        "wallclock(2024-01-01T00:00:00)", // wallclock on non-clock timebase
    ];

    for expr in &bad_exprs {
        assert!(
            time::parse_time_expression(expr, &ctx).is_err(),
            "should reject malformed expression: '{expr}'"
        );
    }
}

#[test]
fn frame_rate_constraint_enforcement() {
    let ctx = time::TimeContext {
        time_base: time::TimeBase::Clock,
        ..Default::default()
    };

    // Frames term is error when timeBase=clock
    assert!(
        time::parse_time_expression("01:02:03:20", &ctx).is_err(),
        "frames term should be rejected when timeBase=clock"
    );
}

#[test]
fn tick_rate_used() {
    let ctx = time::TimeContext {
        tick_rate: 60,
        ..Default::default()
    };
    let expr = time::parse_time_expression("120t", &ctx).unwrap();
    if let time::TimeExpression::OffsetTime { count, metric, .. } = &expr {
        assert_eq!(*count, 120);
        assert_eq!(*metric, time::TimeMetric::T);
    } else {
        panic!("expected OffsetTime");
    }
}

// ─── Style attribute preservation ─────────────────────────────────

#[test]
fn preserve_style_attributes_across_round_trip() {
    let xml = load_fixture("imsc1-backgroundcolor-rgba-001.ttml");
    let mut doc = Document::parse_str(&xml).unwrap();
    normalize_doc(&mut doc);
    let regenerated = doc.to_xml();
    let mut doc2 = Document::parse_str(&regenerated).unwrap();
    normalize_doc(&mut doc2);

    // Check that the region has its style attributes
    if let Some(ref head) = doc2.tt.head
        && let Some(ref layout) = head.layout
        && let Some(first_region) = layout.regions.first()
    {
        assert!(first_region.style_attributes.tts_origin.is_some());
        assert!(first_region.style_attributes.tts_extent.is_some());
    }

    assert_eq!(doc, doc2);
}

// ─── IMSC extension element parsing ───────────────────────────────

#[test]
fn parse_active_area() {
    let xml = load_fixture("imsc1-activearea-001.ttml");
    let doc = Document::parse_str(&xml).unwrap();
    assert_eq!(doc.tt.ittp_active_area.as_deref(), Some("50% 50% 80% 80%"));
    assert_eq!(
        doc.tt
            .head
            .as_ref()
            .unwrap()
            .layout
            .as_ref()
            .unwrap()
            .regions
            .len(),
        3
    );
}

#[test]
fn parse_smpte_background_image() {
    let xml = load_fixture("imsc1-alttext-smpte-backgroundimage-001.ttml");
    let doc = Document::parse_str(&xml).unwrap();
    if let Some(ref body) = doc.tt.body {
        let div = &body.divs[0];
        assert_eq!(
            div.smpte_background_image.as_deref(),
            Some("altText1-img.png")
        );
    } else {
        panic!("no body");
    }
}

#[test]
fn parse_text_shadow_with_negative_offsets() {
    let xml = load_fixture("imsc1.1-textshadow-001.ttml");
    let doc = Document::parse_str(&xml).unwrap();
    // Find the span with textShadow
    if let Some(ref body) = doc.tt.body {
        for div in &body.divs {
            for p in &div.paragraphs {
                for item in &p.content {
                    if let document::InlineContent::Span(span) = item
                        && let Some(ref shadow) = span.style_attributes.tts_text_shadow
                    {
                        assert!(shadow.contains("lime"), "should contain lime color");
                        return;
                    }
                }
            }
        }
    }
    panic!("should find textShadow with negative offsets");
}

// ─── No raw-passthrough check ─────────────────────────────────────

#[test]
fn no_raw_passthrough_in_serializer() {
    // Verify that mutating a document changes its serialized form.
    // A raw-passthrough serializer would ignore mutations and output
    // the original text — so this test fails if the serializer
    // stashes raw input text.
    let xml = load_fixture("imsc1-document-example-822.ttml");
    let mut doc = Document::parse_str(&xml).unwrap();

    let original_output = doc.to_xml();

    // Mutate deeply: change a style attribute on a <p>
    if let Some(ref mut body) = doc.tt.body {
        for div in &mut body.divs {
            for p in &mut div.paragraphs {
                p.style_attributes.tts_color = Some("green".to_string());
            }
        }
    }

    let mutated_output = doc.to_xml();
    assert_ne!(
        original_output, mutated_output,
        "Serializer should produce different output after mutation. \
         If this fails, the serializer may be doing raw-passthrough."
    );

    // Verify the mutation appears in the output
    assert!(
        mutated_output.contains("green"),
        "Mutated output should contain 'green'"
    );
}

#[test]
fn reject_frame_metric_without_frame_rate() {
    // §7.12.7: if the document includes frame terms, ttp:frameRate SHALL be present
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xml:lang="en" xmlns="http://www.w3.org/ns/ttml"
   xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
   ttp:contentProfiles="http://www.w3.org/ns/ttml/profile/imsc1.1/text">
  <body>
    <div>
      <p begin="0f" end="24f">frames without frameRate</p>
    </div>
  </body>
</tt>"#;

    let doc = Document::parse_str(xml).unwrap();
    let validator =
        validation::Validator::new(validation::Profile::Text, validation::ImscVersion::V1_1);
    let result = validator.validate(&doc);
    assert!(
        !result.valid,
        "Document with frame terms but no ttp:frameRate should be rejected"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.constraint.contains("7.12.7")),
        "Should cite §7.12.7 constraint, got: {:?}",
        result.errors
    );
}

// ─── Nesting depth limit tests ───────────────────────────────────────

#[test]
fn rejects_deeply_nested_spans_exceeding_limit() {
    // Generate XML with 65 nested spans (exceeds MAX_NESTING_DEPTH=64).
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en">
  <body>
    <div>
      <p>"#,
    );
    for _ in 0..65 {
        xml.push_str("<span>");
    }
    xml.push_str("text");
    for _ in 0..65 {
        xml.push_str("</span>");
    }
    xml.push_str(
        r#"
      </p>
    </div>
  </body>
</tt>"#,
    );
    let result = Document::parse_str(&xml);
    assert!(
        result.is_err(),
        "Parse should reject 65 nested spans (exceeds limit of 64)"
    );
}

#[test]
fn accepts_shallowly_nested_spans_within_limit() {
    // Generate XML with exactly 64 nested spans (at the limit).
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en">
  <body>
    <div>
      <p>"#,
    );
    for _ in 0..64 {
        xml.push_str("<span>");
    }
    xml.push_str("text");
    for _ in 0..64 {
        xml.push_str("</span>");
    }
    xml.push_str(
        r#"
      </p>
    </div>
  </body>
</tt>"#,
    );
    let result = Document::parse_str(&xml);
    assert!(
        result.is_ok(),
        "Parse should accept 64 nested spans (within limit)"
    );
}

// ─── TT-W1: unknown-namespace content preservation (#1110) ────────

/// One element start as resolved by quick-xml's `NsReader`.
struct SeenElement {
    namespace: String,
    local: String,
    /// `(namespace, local, value)` per attribute (declarations excluded).
    attributes: Vec<(String, String, String)>,
}

/// Walk `xml` with an independent `quick_xml::NsReader` pull loop (the test
/// oracle: it shares no code with the crate's own tree builder), returning every
/// element start in document order plus every trimmed non-empty text run.
fn walk_xml(xml: &str) -> (Vec<SeenElement>, Vec<String>) {
    use quick_xml::NsReader;
    use quick_xml::XmlVersion;
    use quick_xml::escape::resolve_predefined_entity;
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;

    fn ns_of(r: ResolveResult<'_>) -> String {
        match r {
            ResolveResult::Bound(ns) => ns.into_inner().to_string(),
            ResolveResult::Unbound => String::new(),
            ResolveResult::Unknown(p) => panic!("unbound prefix {p}"),
        }
    }

    let mut reader = NsReader::from_str(xml);
    let mut elements = Vec::new();
    let mut texts = Vec::new();
    let mut run = String::new();
    let flush = |run: &mut String, texts: &mut Vec<String>| {
        let t = run.trim();
        if !t.is_empty() {
            texts.push(t.to_string());
        }
        run.clear();
    };
    loop {
        match reader.read_event().expect("well-formed XML") {
            Event::Start(e) | Event::Empty(e) => {
                flush(&mut run, &mut texts);
                let resolver = reader.resolver();
                let (ns, local) = resolver.resolve_element(e.name());
                let mut attributes = Vec::new();
                for a in e.attributes() {
                    let a = a.expect("attribute");
                    let prefix = a.key.prefix();
                    let is_decl = match prefix {
                        None => a.key.as_ref() == "xmlns",
                        Some(p) => p.is_xmlns(),
                    };
                    if is_decl {
                        continue;
                    }
                    let (ans, alocal) = resolver.resolve_attribute(a.key);
                    attributes.push((
                        ns_of(ans),
                        alocal.into_inner().to_string(),
                        a.normalized_value(XmlVersion::Implicit1_0)
                            .expect("value")
                            .into_owned(),
                    ));
                }
                elements.push(SeenElement {
                    namespace: ns_of(ns),
                    local: local.into_inner().to_string(),
                    attributes,
                });
            }
            Event::End(_) | Event::Comment(_) | Event::PI(_) => flush(&mut run, &mut texts),
            Event::Text(t) => run.push_str(&t.xml10_content()),
            Event::CData(c) => run.push_str(&c.xml10_content()),
            Event::GeneralRef(r) => match r.resolve_char_ref().expect("char ref") {
                Some(c) => run.push(c),
                None => run.push_str(resolve_predefined_entity(&r).expect("entity")),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    flush(&mut run, &mut texts);
    (elements, texts)
}

/// The namespace URI the root element binds `prefix` to, if any.
fn root_prefix_binding(xml: &str, prefix: &str) -> Option<String> {
    use quick_xml::NsReader;
    use quick_xml::events::Event;
    use quick_xml::name::{QName, ResolveResult};

    let mut reader = NsReader::from_str(xml);
    loop {
        match reader.read_event().expect("well-formed XML") {
            Event::Start(_) | Event::Empty(_) => {
                let probe = format!("{prefix}:x");
                return match reader.resolver().resolve_element(QName(&probe)).0 {
                    ResolveResult::Bound(ns) => Some(ns.into_inner().to_string()),
                    _ => None,
                };
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// Canonical, whitespace- and prefix-independent dump of a parsed XML tree:
/// sorted lines of `el <uri>|<local>`, `at <uri>|<local>|<value>` and trimmed
/// text. Two documents that are namespace-equivalent dump identically.
fn canonical_dump(xml: &str) -> Vec<String> {
    let (elements, texts) = walk_xml(xml);
    let mut lines = Vec::new();
    for el in elements {
        lines.push(format!("el {}|{}", el.namespace, el.local));
        for (ns, local, value) in el.attributes {
            lines.push(format!("at {ns}|{local}|{value}"));
        }
    }
    lines.extend(texts.into_iter().map(|t| format!("tx {t}")));
    lines.sort();
    lines
}

const FOREIGN_TT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml"
    xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
    xmlns:acme="urn:acme:ext"
    xmlns:ttm="http://www.w3.org/ns/ttml#metadata"
    xml:lang="en">
  <metadata acme:rating="PG" acme:nested-thing="1">
    <acme:thing acme:id="t1">hello<acme:inner>deep</acme:inner></acme:thing>
  </metadata>
  <body>
    <div acme:zone="a">
      <p begin="0s" end="5s" acme:cue="7"><span acme:inline="i1">Hello</span><br acme:br="b1"/><acme:para>px</acme:para></p>
    </div>
  </body>
</tt>"#;

#[test]
fn unknown_namespace_content_round_trips() {
    let mut doc = Document::parse_str(FOREIGN_TT).unwrap();
    let xml2 = doc.to_xml();
    // Parse-equivalence (not string equality): same elements, attributes,
    // namespaces and text on both sides.
    assert_eq!(canonical_dump(FOREIGN_TT), canonical_dump(&xml2));
}

#[test]
fn unknown_namespace_prefix_binding_is_preserved() {
    let mut doc = Document::parse_str(FOREIGN_TT).unwrap();
    let xml2 = doc.to_xml();
    // The original `acme` prefix must still be bound to the same URI in the
    // serialized document.
    let acme = root_prefix_binding(&xml2, "acme").expect("acme prefix still declared");
    assert_eq!(acme, "urn:acme:ext");
}

#[test]
fn unknown_tt_namespace_element_preserved() {
    // A ttNamespaces-external element inside <metadata> (e.g. EBU-TT style)
    // must survive as an Unknown child, not be dropped.
    let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ebuttm="urn:ebu:tt:meta" xml:lang="en">
  <metadata><ebuttm:documentMetadata><ebuttm:conformsToStandard>nope</ebuttm:conformsToStandard></ebuttm:documentMetadata><ebuttm:extra>x</ebuttm:extra></metadata>
  <body><div><p begin="0s" end="1s">hi</p></div></body>
</tt>"#;
    let mut doc = Document::parse_str(xml).unwrap();
    let out = doc.to_xml();
    assert_eq!(canonical_dump(xml), canonical_dump(&out));
}

#[test]
fn prefix_collision_gets_fallback_binding() {
    // The same prefix bound to two URIs: the second URI cannot reuse the
    // prefix, so it must get a generated ttmfallbackN binding instead of
    // silently corrupting either.
    let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:v="urn:one" xml:lang="en">
  <body><div>
    <p begin="0s" end="1s"><span xmlns:v="urn:two" v:x="2">t</span></p>
  </div></body>
</tt>"#;
    let mut doc = Document::parse_str(xml).unwrap();
    let out = doc.to_xml();
    // `v:x` is the only content that resolved to `urn:two`; `xmlns:`
    // declarations are not attributes, so the surviving evidence is the
    // attribute's own namespace URI.
    let (elements, _) = walk_xml(&out);
    let has_two = elements
        .iter()
        .flat_map(|e| e.attributes.iter())
        .any(|(ns, _, _)| ns == "urn:two");
    assert!(has_two, "urn:two content must survive");
    assert_eq!(canonical_dump(xml), canonical_dump(&out));
}

#[test]
fn foreign_attributes_on_content_elements_are_typed_and_preserved() {
    // TT-W1: `<div>`/`<p>`/`<span>`/`<br>` carry foreign attributes into the
    // typed fields (not just metadata elements), and every one of them is
    // re-emitted — the pre-#1110 serializer dropped all of them.
    let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:acme="urn:acme:ext" xml:lang="en">
  <body><div acme:zone="a">
    <p begin="0s" end="1s" acme:cue="7"><span acme:inline="i1">H</span><br acme:br="b1"/><acme:para>px</acme:para></p>
  </div></body>
</tt>"#;
    let mut doc = Document::parse_str(xml).unwrap();
    let div = &doc.tt.body.as_ref().unwrap().divs[0];
    assert_eq!(div.foreign_attributes.len(), 1);
    assert_eq!(div.foreign_attributes[0].local_name, "zone");
    assert_eq!(div.foreign_attributes[0].value, "a");
    assert_eq!(
        div.foreign_attributes[0].namespace.as_deref(),
        Some("urn:acme:ext")
    );
    assert_eq!(div.foreign_attributes[0].prefix.as_deref(), Some("acme"));
    let p = &div.paragraphs[0];
    assert_eq!(p.foreign_attributes[0].local_name, "cue");
    assert!(matches!(
        p.content.first(),
        Some(document::InlineContent::Span(span)) if span.foreign_attributes[0].local_name == "inline"
    ));
    assert!(matches!(
        p.content.get(1),
        Some(document::InlineContent::Br(br)) if br.foreign_attributes[0].local_name == "br"
    ));

    let out = doc.to_xml();
    assert_eq!(canonical_dump(xml), canonical_dump(&out));
}

#[test]
fn unmodeled_tt_namespace_element_survives_in_body() {
    // TT-W1: an unrecognized element in the TT namespace itself (TTML2 §7.3)
    // is kept as an unknown subtree rather than dropped.
    let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tt="http://www.w3.org/ns/ttml" xml:lang="en">
  <body><div><tt:future begin="0s" end="1s">x</tt:future></div></body>
</tt>"#;
    let mut doc = Document::parse_str(xml).unwrap();
    let div = &doc.tt.body.as_ref().unwrap().divs[0];
    assert_eq!(div.unknown_children.len(), 1);
    assert_eq!(div.unknown_children[0].local_name, "future");
    assert_eq!(
        div.unknown_children[0].namespace.as_deref(),
        Some(document::NS_TT)
    );
    let out = doc.to_xml();
    assert_eq!(canonical_dump(xml), canonical_dump(&out));
}

// ─── TT-W2: IMSC image / resource elements (#1110) ────────────────

#[test]
fn round_trip_image_elements_imsc() {
    let xml = load_fixture("imsc1.1-image-profile-001.ttml");
    let mut doc = Document::parse_str(&xml).unwrap();
    // The div-level <image> is typed with src/type/extent.
    let body = doc.tt.body.as_ref().unwrap();
    let imgs: Vec<_> = body.divs.iter().flat_map(|d| &d.images).collect();
    assert!(!imgs.is_empty(), "fixture must carry a typed image");
    let img = imgs
        .iter()
        .find(|i| i.src.as_deref() == Some("image001-img.png"))
        .expect("image src");
    assert_eq!(img.type_.as_deref(), Some("image/png"));
    assert!(img.tts_extent.is_some(), "tts:extent preserved");
    // Round-trip keeps the image intact.
    let out = doc.to_xml();
    assert_eq!(canonical_dump(&xml), canonical_dump(&out));
}

#[test]
fn round_trip_smpte_background_image() {
    let xml = load_fixture("imsc1-alttext-smpte-backgroundimage-001.ttml");
    let mut doc = Document::parse_str(&xml).unwrap();
    let out = doc.to_xml();
    let reparsed = Document::parse_str(&out).unwrap();
    let body = reparsed.tt.body.unwrap();
    assert_eq!(
        body.divs[0].smpte_background_image.as_deref(),
        Some("altText1-img.png")
    );
    assert_eq!(canonical_dump(&xml), canonical_dump(&out));
}

#[test]
fn inline_image_and_audio_in_paragraph_round_trip() {
    // TTML2 §8.1.4/§9.3: <image> and <audio> are inline content of <p>.
    let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xml:lang="en">
  <body><div>
    <p begin="0s" end="2s">a<image src="x.png" type="image/png"/><audio src="y.wav" type="audio/wav">z</audio>b</p>
  </div></body>
</tt>"#;
    let mut doc = Document::parse_str(xml).unwrap();
    let body = doc.tt.body.as_ref().unwrap();
    let p = &body.divs[0].paragraphs[0];
    assert!(
        p.content.iter().any(
            |c| matches!(c, document::InlineContent::Image(i) if i.src.as_deref() == Some("x.png"))
        ),
        "typed inline image"
    );
    assert!(
        p.content.iter().any(
            |c| matches!(c, document::InlineContent::Audio(a) if a.src.as_deref() == Some("y.wav"))
        ),
        "typed inline audio"
    );
    let out = doc.to_xml();
    assert_eq!(canonical_dump(xml), canonical_dump(&out));
}

#[test]
fn resources_font_and_source_round_trip() {
    // TTML2 §9.1: <resources> with <font> and nested <source><data>.
    let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en">
  <head>
    <resources>
      <font xml:id="f1" family="Foo" src="foo.otf" type="font/opentype" style="italic" weight="bold" range="U+0041">
        <source>
          <data encoding="base64" length="4">QkJC</data>
        </source>
      </font>
    </resources>
  </head>
  <body><div><p begin="0s" end="1s">hi</p></div></body>
</tt>"#;
    let mut doc = Document::parse_str(xml).unwrap();
    let head = doc.tt.head.as_ref().unwrap();
    let resources = head.resources.as_ref().expect("resources element");
    let font = &resources.fonts[0];
    assert_eq!(font.family.as_deref(), Some("Foo"));
    assert_eq!(font.style_.as_deref(), Some("italic"));
    let src = &font.sources[0];
    let data = src.data.as_ref().expect("source data");
    assert_eq!(data.encoding.as_deref(), Some("base64"));
    assert_eq!(data.text.as_deref(), Some("QkJC"));
    let out = doc.to_xml();
    assert_eq!(canonical_dump(xml), canonical_dump(&out));
}
