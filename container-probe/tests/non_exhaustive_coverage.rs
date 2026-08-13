//! Drift-guard: every `pub enum` in container-probe must carry
//! `#[non_exhaustive]` (issue #806), AND every data-carrying (struct/tuple)
//! variant of a non-exhaustive `pub enum` must carry its own
//! `#[non_exhaustive]`.
//!
//! The first half scans `src/` for every `pub enum` and fails if any non-SKIP
//! enum lacks a `#[non_exhaustive]` attribute on one of the five lines
//! immediately preceding the `pub enum` declaration.
//!
//! The second half is the variant-level counterpart: the old scan was
//! structurally blind to variant drift — it checked only lines starting
//! `pub enum `, so a struct variant like `Probe::Ambiguous { .. }` could lose
//! its `#[non_exhaustive]` (or a new one could be added without it) and the
//! guard stayed green. Each data-carrying variant of a non-exhaustive enum must
//! therefore carry `#[non_exhaustive]` on the line immediately above its
//! identifier.
//!
//! `#[non_exhaustive]` is not required on unit variants (nothing to add), so the
//! scan keys off the `{`/`(` that marks a data-carrying variant.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Enums that are intentionally exempt from the `#[non_exhaustive]` requirement.
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
    let mut missing_variant: BTreeSet<String> = BTreeSet::new();

    for (_path, content) in &files {
        let lines: Vec<&str> = content.lines().collect();
        let mut i = 0usize;
        while i < lines.len() {
            let trimmed = lines[i].trim_start();
            if let Some(rest) = trimmed.strip_prefix("pub enum ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() && !SKIP.contains(&name.as_str()) {
                    // Check up to 5 lines above for #[non_exhaustive]
                    let start = i.saturating_sub(5);
                    let has_ne = lines[start..i]
                        .iter()
                        .any(|l| l.trim_start().starts_with("#[non_exhaustive]"));
                    if !has_ne {
                        missing.insert(name.clone());
                    }

                    // Walk the enum body and check each data-carrying variant.
                    i = check_variant_non_exhaustive(&lines, i, &name, &mut missing_variant);
                    continue;
                }
            }
            i += 1;
        }
    }

    assert!(
        missing.is_empty(),
        "pub enum(s) missing `#[non_exhaustive]`: {missing:?}\n\
         Add `#[non_exhaustive]` immediately before the `pub enum` declaration, \
         or add the enum to SKIP with a reason if it genuinely cannot be non-exhaustive."
    );
    assert!(
        missing_variant.is_empty(),
        "data-carrying enum variant(s) missing `#[non_exhaustive]`: {missing_variant:?}\n\
         Add `#[non_exhaustive]` immediately above the variant so its fields \
         remain non-constructible/non-matchable without `..` for downstream crates."
    );
}

/// Scan the body of a `pub enum` starting at the line holding `pub enum ...`,
/// reporting any data-carrying variant (one whose identifier is followed by a
/// `{` or `(` on the same line) that lacks `#[non_exhaustive]` on the line
/// directly above its identifier. Returns the index of the line after the
/// enum's closing `}`.
fn check_variant_non_exhaustive(
    lines: &[&str],
    enum_line: usize,
    enum_name: &str,
    missing: &mut BTreeSet<String>,
) -> usize {
    // Find the opening brace of the enum body.
    let mut i = enum_line;
    while i < lines.len() && !lines[i].contains('{') {
        i += 1;
    }
    if i >= lines.len() {
        return enum_line + 1;
    }

    // `depth` is the brace nesting level *before* processing the current line.
    // 0 = outside the enum, 1 = directly inside the enum body, >=2 = inside a
    // nested brace block (a struct variant's field list). A data-carrying
    // variant declaration therefore only appears while depth == 1 and the line
    // itself carries a `{` or `(`.
    let mut depth = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        let opens = line.matches('{').count();
        let closes = line.matches('}').count();

        if depth == 1 {
            let has_field_marker = t.contains('{') || t.contains('(');
            let is_meta = t.starts_with('#')
                || t.starts_with("///")
                || t.starts_with("//")
                || t.starts_with('}');
            if has_field_marker && !is_meta {
                let marker_pos = t.find('{').or_else(|| t.find('(')).unwrap_or(0);
                let ident = t[..marker_pos].trim();
                let variant_name = ident.split_whitespace().next().unwrap_or("");
                let has_ne = i > 0 && lines[i - 1].trim_start().starts_with("#[non_exhaustive]");
                if !has_ne && !variant_name.is_empty() {
                    missing.insert(format!("{enum_name}::{variant_name}"));
                }
            }
        }

        depth = depth + opens - closes;
        if depth == 0 {
            // Enum body closed on this line.
            return i + 1;
        }
        i += 1;
    }
    enum_line + 1
}
