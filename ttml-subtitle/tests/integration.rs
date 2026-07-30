//! Integration tests: parse all 11 real IMSC fixtures, round-trip,
//! profile validation, time expression exhaustive tests.

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
    // Normalize root text
    if let Some(ref text) = doc.tt.text {
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            doc.tt.text = None;
        } else {
            doc.tt.text = Some(trimmed);
        }
    }
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
    if let Some(ref head) = doc2.tt.head {
        if let Some(ref layout) = head.layout {
            if let Some(first_region) = layout.regions.first() {
                assert!(first_region.style_attributes.tts_origin.is_some());
                assert!(first_region.style_attributes.tts_extent.is_some());
            }
        }
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
                    if let document::InlineContent::Span(span) = item {
                        if let Some(ref shadow) = span.style_attributes.tts_text_shadow {
                            assert!(shadow.contains("lime"), "should contain lime color");
                            return;
                        }
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
