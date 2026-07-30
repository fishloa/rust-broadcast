//! Parse a simple TTML document and print its structure.
//!
//! Usage: `cargo run -p ttml-subtitle --example parse_document`

use std::fs;
use std::path::PathBuf;

use ttml_subtitle::Document;

fn main() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("imsc1-document-example-822.ttml");

    let xml = fs::read_to_string(&fixture).expect("failed to read fixture");

    let doc = Document::parse_str(&xml).expect("failed to parse document");

    println!("Document language: {:?}", doc.tt.xml_lang);
    println!("Profile: {:?}", doc.tt.ttp_profile);

    if let Some(ref head) = doc.tt.head {
        for meta in &head.metadata {
            if let ttml_subtitle::document::MetadataChild::TtmTitle(t) = meta {
                println!("Title: {}", t.text);
            }
        }
    }

    if let Some(ref body) = doc.tt.body {
        for div in &body.divs {
            for p in &div.paragraphs {
                let begin = p.begin.as_deref().unwrap_or("(no begin)");
                let end = p.end.as_deref().unwrap_or("(no end)");
                print!("  [{begin} → {end}] ");
                for item in &p.content {
                    match item {
                        ttml_subtitle::document::InlineContent::Text(text) => {
                            print!("{text}");
                        }
                        ttml_subtitle::document::InlineContent::Span(span) => {
                            let color = span
                                .style_attributes
                                .tts_background_color
                                .as_deref()
                                .unwrap_or("default");
                            let span_text: String = span
                                .content
                                .iter()
                                .filter_map(|c| {
                                    if let ttml_subtitle::document::InlineContent::Text(t) = c {
                                        Some(t.as_str())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("");
                            print!("[span({color}): {span_text}]");
                        }
                        ttml_subtitle::document::InlineContent::Br(_) => {
                            println!();
                        }
                        _ => {}
                    }
                }
                println!();
            }
        }
    }
}
