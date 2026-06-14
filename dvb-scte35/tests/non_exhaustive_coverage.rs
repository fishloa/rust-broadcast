//! Drift-guard: every `pub enum` in dvb-scte35 must carry `#[non_exhaustive]`.
//!
//! Scans `src/` for every `pub enum`, subtracts a documented SKIP list, and
//! fails if any remaining enum lacks a `#[non_exhaustive]` attribute on one of
//! the five lines immediately preceding the `pub enum` declaration.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Enums that are intentionally exempt from the `#[non_exhaustive]` requirement.
///
/// This list is empty: every public enum in dvb-scte35 should be non-exhaustive.
const SKIP: &[&str] = &[];

fn read_rs_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            read_rs_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            let content = fs::read_to_string(&path).expect("read .rs");
            out.push((path.display().to_string(), content));
        }
    }
}

#[test]
fn every_public_enum_is_non_exhaustive() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    read_rs_files(&src, &mut files);

    let mut missing: BTreeSet<String> = BTreeSet::new();

    for (_path, content) in &files {
        let lines: Vec<&str> = content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("pub enum ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name.is_empty() || SKIP.contains(&name.as_str()) {
                    continue;
                }
                // Check up to 5 lines above for #[non_exhaustive]
                let start = i.saturating_sub(5);
                let has_ne = lines[start..i]
                    .iter()
                    .any(|l| l.trim_start().starts_with("#[non_exhaustive]"));
                if !has_ne {
                    missing.insert(name);
                }
            }
        }
    }

    assert!(
        missing.is_empty(),
        "pub enum(s) missing `#[non_exhaustive]`: {missing:?}\n\
         Add `#[non_exhaustive]` immediately before the `pub enum` declaration, \
         or add the enum to SKIP with a reason if it genuinely cannot be non-exhaustive."
    );
}
