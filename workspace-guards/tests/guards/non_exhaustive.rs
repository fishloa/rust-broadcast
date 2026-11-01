//! Drift-guard: every `pub enum` in a table crate must carry
//! `#[non_exhaustive]` on one of the five lines immediately preceding the
//! declaration (issue #806). Enums named in a crate's `skip` are exempt.
//!
//! `variants: true` (container-probe) adds the variant-level counterpart: every
//! data-carrying (struct/tuple) variant of a `pub enum` must carry its own
//! `#[non_exhaustive]` on the line immediately above its identifier. The plain
//! scan is blind to variant drift (it keys off lines starting `pub enum `), so a
//! struct variant like `Probe::Ambiguous { .. }` could lose its attribute while
//! the guard stayed green. `#[non_exhaustive]` is not required on unit variants.

use crate::common::assert_table_covers_members;
use std::collections::BTreeSet;
use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    pub skip: &'static [&'static str],
    pub variants: bool,
}

const fn plain(name: &'static str) -> Crate {
    Crate {
        name,
        skip: &[],
        variants: false,
    }
}

fn enum_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("pub enum ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn has_non_exhaustive_above(lines: &[&str], i: usize) -> bool {
    let start = i.saturating_sub(5);
    lines[start..i]
        .iter()
        .any(|l| l.trim_start().starts_with("#[non_exhaustive]"))
}

fn scan_enums(c: &Crate, path: &str, content: &str, missing: &mut BTreeSet<String>) {
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let Some(name) = enum_name(line.trim_start()) else {
            continue;
        };
        if c.skip.contains(&name.as_str()) {
            continue;
        }
        if !has_non_exhaustive_above(&lines, i) {
            missing.insert(format!("{}: {path}:{}: pub enum {name}", c.name, i + 1));
        }
    }
}

fn scan_enums_and_variants(
    c: &Crate,
    path: &str,
    content: &str,
    missing: &mut BTreeSet<String>,
    missing_variant: &mut BTreeSet<String>,
) {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if let Some(name) = enum_name(trimmed)
            && !c.skip.contains(&name.as_str())
        {
            if !has_non_exhaustive_above(&lines, i) {
                missing.insert(format!("{}: {path}:{}: pub enum {name}", c.name, i + 1));
            }
            let mut found = BTreeSet::new();
            i = check_variant_non_exhaustive(&lines, i, &name, &mut found);
            for v in found {
                missing_variant.insert(format!("{}: {path}: {v}", c.name));
            }
            continue;
        }
        i += 1;
    }
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
            return i + 1;
        }
        i += 1;
    }
    enum_line + 1
}

#[test]
fn every_public_enum_is_non_exhaustive() {
    let mut missing = BTreeSet::new();
    let mut missing_variant = BTreeSet::new();
    for c in crate::non_exhaustive_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        for (path, content) in &files {
            if c.variants {
                scan_enums_and_variants(c, path, content, &mut missing, &mut missing_variant);
            } else {
                scan_enums(c, path, content, &mut missing);
            }
        }
    }
    assert!(
        missing.is_empty(),
        "pub enum(s) missing `#[non_exhaustive]`:\n{}\n\
         Add `#[non_exhaustive]` immediately before the `pub enum` declaration, \
         or add the enum to the crate's `skip` in non_exhaustive_data.rs with a reason \
         if it genuinely cannot be non-exhaustive.",
        missing.into_iter().collect::<Vec<_>>().join("\n")
    );
    assert!(
        missing_variant.is_empty(),
        "data-carrying enum variant(s) missing `#[non_exhaustive]`:\n{}\n\
         Add `#[non_exhaustive]` immediately above the variant so its fields \
         remain non-constructible/non-matchable without `..` for downstream crates.",
        missing_variant.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn table_covers_every_workspace_member() {
    let names: Vec<&str> = crate::non_exhaustive_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    assert_table_covers_members("non_exhaustive", &names, crate::non_exhaustive_data::EXEMPT);
}

#[test]
fn detector_bites() {
    let c = plain("synthetic");
    let mut missing = BTreeSet::new();
    scan_enums(
        &c,
        "src/x.rs",
        "#[non_exhaustive]\npub enum Ok1 { A }\n\n\n\n\n\n\npub enum Bad { A }\n",
        &mut missing,
    );
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert!(
        missing
            .iter()
            .next()
            .unwrap()
            .contains("src/x.rs:9: pub enum Bad")
    );

    let (mut m, mut mv) = (BTreeSet::new(), BTreeSet::new());
    let v = Crate {
        name: "synthetic",
        skip: &[],
        variants: true,
    };
    scan_enums_and_variants(
        &v,
        "src/y.rs",
        "#[non_exhaustive]\npub enum E {\n    Unit,\n    Data {\n        x: u8,\n    },\n}\n",
        &mut m,
        &mut mv,
    );
    assert!(m.is_empty(), "{m:?}");
    assert_eq!(mv.len(), 1, "{mv:?}");
}
