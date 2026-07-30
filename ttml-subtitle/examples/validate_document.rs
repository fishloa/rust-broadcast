//! Validate a TTML document against an IMSC profile.
//!
//! Usage: `cargo run -p ttml-subtitle --example validate_document`

use std::fs;
use std::path::PathBuf;

use ttml_subtitle::{
    Document,
    validation::{ImscVersion, Profile, Validator},
};

fn main() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("imsc1.1-ruby-001.ttml");

    let xml = fs::read_to_string(&fixture).expect("failed to read fixture");

    let doc = Document::parse_str(&xml).expect("failed to parse document");

    // Validate as IMSC 1.1 Text Profile
    let validator = Validator::new(Profile::Text, ImscVersion::V1_1);
    let result = validator.validate(&doc);

    if result.valid {
        println!("Document is valid IMSC 1.1 Text Profile.");
    } else {
        println!("Validation errors:");
        for err in &result.errors {
            println!("  - {}: {}", err.constraint, err.detail);
        }
    }

    // Try validating as Image Profile (should fail — ruby is not in Image Profile)
    let img_validator = Validator::new(Profile::Image, ImscVersion::V1_1);
    let img_result = img_validator.validate(&doc);
    println!(
        "Image Profile check: {}",
        if img_result.valid {
            "valid"
        } else {
            "non-conformant (expected)"
        }
    );
}
