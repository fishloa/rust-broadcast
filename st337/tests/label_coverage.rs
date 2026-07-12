//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list (error enums that carry no spec label), and fails if any
//! remaining enum has neither `broadcast_common::impl_spec_display!(Name)`
//! nor a hand-written `Display` impl.
//!
//! Because the project-wide `Display` impl delegates to an inherent
//! `name() -> &'static str`, a present `Display` transitively guarantees
//! `name()` exists (it would not compile otherwise) — so this single coverage
//! check enforces the whole convention and catches the one thing the compiler
//! cannot: a brand-new `pub enum` that nobody labelled.
//!
//! `st337` defines exactly one spec/field `pub enum`, [`st337::DataMode`]
//! (SMPTE ST 337 §7.2.4.3 Table 8), which carries `name()` +
//! `impl_spec_display!(DataMode)`. `data_type` is deliberately **not** an
//! enum (see `docs/st337.md` scope decision 3 — the type -> codec mapping is
//! registered in SMPTE ST 338, not independently verified here), so this
//! test also guards against a future contributor introducing an unlabelled
//! `data_type` enum without reading that decision first.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Enums that are intentionally **not** spec/field labels: the structured
/// error type has no spec label.
const SKIP: &[&str] = &["Error"];

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

/// True if `name` appears after `prefix` with an identifier boundary.
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

    let missing: Vec<_> = enums
        .iter()
        .filter(|e| !SKIP.contains(&e.as_str()))
        .filter(|e| !has_impl(&all, "impl_spec_display!(", e) && !has_impl(&all, "Display for ", e))
        .cloned()
        .collect();

    assert!(
        missing.is_empty(),
        "pub enum(s) missing a Display impl (issue #204 convention): {missing:?}\n\
         Add `broadcast_common::impl_spec_display!(Name)` plus an inherent `name()`, \
         or add the enum to SKIP if it is not a spec/field label."
    );
}

#[test]
fn data_mode_is_the_only_spec_enum() {
    // A cheap sanity check that the crate hasn't grown a second enum without
    // anyone updating this file's doc comment.
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
    let non_skip: Vec<_> = enums
        .iter()
        .filter(|e| !SKIP.contains(&e.as_str()))
        .collect();
    assert_eq!(
        non_skip,
        vec!["DataMode"],
        "expected DataMode to be the only spec/field enum; if you added a new one, \
         label it (see this file's doc comment) and update this assertion"
    );
}
