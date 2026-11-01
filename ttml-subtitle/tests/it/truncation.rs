//! Truncation gate: cutting a committed fixture at ANY offset before the end of
//! its root end tag must be a structured `Error::XmlParse`. The only accepted
//! prefixes are the complete document plus trailing whitespace, asserted
//! explicitly as `Ok`.
#![cfg(feature = "std")]

use std::fs;
use std::path::PathBuf;

use ttml_subtitle::{Document, Error};

#[test]
fn every_fixture_truncation_errors() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("fixtures dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "ttml") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read fixture");
        let name = path.display().to_string();
        let complete = text.trim_end().len();
        for end in 1..text.len() {
            if !text.is_char_boundary(end) {
                continue;
            }
            let result = Document::parse_str(&text[..end]);
            if end < complete {
                assert!(
                    matches!(result, Err(Error::XmlParse(_))),
                    "{name}: truncation at byte {end}/{} must be Err(XmlParse), got {:?}",
                    text.len(),
                    result.map(|_| ())
                );
            } else {
                assert!(
                    result.is_ok(),
                    "{name}: prefix of {end} bytes is the complete document (plus whitespace) and must parse"
                );
            }
        }
        assert!(
            Document::parse_str(&text).is_ok(),
            "{name}: full document parses"
        );
        checked += 1;
    }
    assert_eq!(checked, 11, "all committed .ttml fixtures were checked");
}
