//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list, and fails if any remaining enum has neither
//! `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
//! impl. A present `Display` delegates to an inherent `name()`, so this one
//! check transitively enforces the whole convention (issue #806).
//!
//! Skip list:
//! - `Error` — structured `thiserror` error, not a spec/field label.
//! - `InlineContent` — a data-carrying ADT with typed variants (Text, Span, Br);
//!   callers match the typed variant; a static label would be lossy.
//! - `AnimationChild` — a data-carrying ADT with typed variants.
//! - `MetadataChild` — a data-carrying ADT with typed variants.
//! - `TimeExpression` — a data-carrying ADT with different time forms;
//!   callers match the variant; a static label would be lossy.
//! - `WallclockForm` — a data-carrying ADT with typed variants.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SKIP: &[&str] = &[
    "Error",
    "InlineContent",
    "AnimationChild",
    "MetadataChild",
    "TimeExpression",
    "WallclockForm",
];

fn read_rs(dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            read_rs(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(fs::read_to_string(&path).expect("read .rs"));
        }
    }
}

/// True if `{prefix}{name}` appears with an identifier boundary (whole name).
fn has_impl(all: &str, prefix: &str, name: &str) -> bool {
    let needle = format!("{prefix}{name}");
    let mut start = 0;
    while let Some(idx) = all[start..].find(&needle) {
        let end = start + idx + needle.len();
        let next = all[end..].chars().next();
        if !matches!(next, Some(c) if c.is_alphanumeric() || c == '_') {
            return true;
        }
        start = end;
    }
    false
}

#[test]
fn every_public_spec_enum_has_a_display_impl() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    read_rs(&src, &mut files);
    let all = files.join("\n");

    let mut enums = BTreeSet::new();
    for line in all.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("pub enum ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                enums.insert(name);
            }
        }
    }

    let mut missing = Vec::new();
    for name in &enums {
        if SKIP.contains(&name.as_str()) {
            continue;
        }
        let labelled = has_impl(&all, "impl_spec_display!(", name)
            || has_impl(&all, "impl ::core::fmt::Display for ", name)
            || has_impl(&all, "impl core::fmt::Display for ", name)
            || has_impl(&all, "impl std::fmt::Display for ", name)
            || has_impl(&all, "impl fmt::Display for ", name);
        if !labelled {
            missing.push(name.clone());
        }
    }

    assert!(
        missing.is_empty(),
        "public enum(s) missing a #204 Display/name() label (or a SKIP-list entry): {missing:?}"
    );
}
