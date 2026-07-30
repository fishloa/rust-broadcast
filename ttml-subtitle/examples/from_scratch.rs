//! Build a TTML document from a minimal shell and customize it.
//!
//! Since TTML element structs are `#[non_exhaustive]`, external callers
//! cannot construct them with field literals. The intended authoring pattern
//! is to parse a minimal template and mutate its fields.
//!
//! Usage: `cargo run -p ttml-subtitle --example from_scratch`

use ttml_subtitle::{
    Document,
    validation::{ImscVersion, Profile, Validator},
};

fn main() {
    // Start from a minimal document shell
    let template = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xml:lang="en" xmlns="http://www.w3.org/ns/ttml"
   xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
   xmlns:tts="http://www.w3.org/ns/ttml#styling">
  <head><layout><region xml:id="r1" tts:extent="80% 80%" tts:origin="10% 10%"/></layout></head>
  <body><div><p region="r1" begin="0s" end="5s">Hello</p></div></body>
</tt>"#;

    let mut doc = Document::parse_str(template).expect("parse template");

    // Customize: add content profile
    doc.tt.ttp_content_profiles =
        Some("http://www.w3.org/ns/ttml/profile/imsc1.1/text".to_string());

    // Mutate the paragraph text
    if let Some(ref mut body) = doc.tt.body {
        for div in &mut body.divs {
            for p in &mut div.paragraphs {
                p.content.clear();
                p.content.push(ttml_subtitle::InlineContent::Text(
                    "Built from template, mutated programmatically!".to_string(),
                ));
            }
        }
    }

    println!("{}", doc.to_xml());

    // Validate
    let validator = Validator::new(Profile::Text, ImscVersion::V1_1);
    let result = validator.validate(&doc);
    println!("Valid IMSC 1.1 Text Profile: {}", result.valid);
}
