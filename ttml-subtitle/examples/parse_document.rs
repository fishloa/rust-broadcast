//! Parse a simple TTML document and print its structure.
//!
//! Usage: `cargo run -p ttml-subtitle --example parse_document`

use broadcast_common::bounded::{Limit, read_file_bounded_string};

use ttml_subtitle::Document;

/// A whole TTML document, bounded by the ceiling the XML reader accepts for
/// any document this crate parses.
const DOCUMENT_LIMIT: Limit = Limit::new(
    "TTML document",
    broadcast_common::xml_chars::MAX_XML_BYTES as u64,
);

fn main() {
    let fixture = test_fixtures::path("subtitles/imsc1-document-example-822.ttml");

    let xml = read_file_bounded_string(&fixture, DOCUMENT_LIMIT).expect("failed to read fixture");

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
