//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list, and fails if any remaining enum has neither
//! `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
//!
//! Because the project-wide `Display` impl delegates to an inherent
//! `name() -> &'static str`, a present `Display` transitively guarantees
//! `name()` exists (it would not compile otherwise) — so this single coverage
//! check enforces the whole convention and catches the one thing the compiler
//! cannot: a brand-new `pub enum` that nobody labelled.
//!
//! Public enums in cc-data and their status:
//! - `CcType` — spec/field label → has `name()` + `impl_spec_display!`
//! - `Cea608Mode` — spec/field label → has `name()` + `impl_spec_display!`
//! - `Cea608Channel` — spec/field label → has `name()` + `impl_spec_display!`
//! - `Cea608Color` — spec/field label (CTA-608-E Tables 51/53) → has `name()` + `impl_spec_display!`
//! - `WindowState` — spec/field label → has `name()` + `impl_spec_display!`
//! - `AnchorPoint` — spec/field label (CTA-708-E §8.4.6) → has `name()` + `impl_spec_display!`
//! - `Opacity` — spec/field label → has `name()` + `impl_spec_display!`
//! - `EdgeType` — spec/field label → has `name()` + `impl_spec_display!`
//! - `PenSize` — spec/field label → has `name()` + `impl_spec_display!`
//! - `PenOffset` — spec/field label → has `name()` + `impl_spec_display!`
//! - `FontStyle` — spec/field label → has `name()` + `impl_spec_display!`
//! - `Justify` — spec/field label → has `name()` + `impl_spec_display!`
//! - `PrintDirection` — spec/field label → has `name()` + `impl_spec_display!`
//! - `ScrollDirection` — spec/field label → has `name()` + `impl_spec_display!`
//!
//! SKIP list:
//! - `Error` — structured error ADT, not a spec/field label.
//! - `WindowMapOp` — internal operation discriminant, not a public wire field.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Enums that are intentionally **not** spec/field labels. Each is one of:
/// a structured error, an internal operation discriminant, or a data-carrying
/// ADT whose variants hold payloads.
const SKIP: &[&str] = &[
    // structured error
    "Error",
    // internal window-map operation discriminant — not pub, not a wire field
    "WindowMapOp",
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

/// True if `name` appears after `prefix` with an identifier boundary, i.e. the
/// match is the whole enum name and not a longer one sharing the prefix.
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
