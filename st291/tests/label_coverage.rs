//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list (error / dispatch-wrapper enums that carry no spec label), and
//! fails if any remaining enum has neither
//! `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display` impl.
//!
//! Because the project-wide `Display` impl delegates to an inherent
//! `name() -> &'static str`, a present `Display` transitively guarantees
//! `name()` exists (it would not compile otherwise) — so this single coverage
//! check enforces the whole convention and catches the one thing the compiler
//! cannot: a brand-new `pub enum` that nobody labelled.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Enums that are intentionally **not** spec/field labels: the structured
/// error type has no spec label, and any future dispatch wrappers belong here.
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
    let is_boundary =
        |rest: &str| !matches!(rest.chars().next(), Some(c) if c.is_alphanumeric() || c == '_');
    let is_path_or_space =
        |c: char| c.is_whitespace() || c == ':' || c.is_alphanumeric() || c == '_';

    // Strip a leading `ident::` chain (`crate::`, `broadcast_common::`, …) so a
    // re-exported or fully-qualified invocation still counts as reaching the
    // needle from the very start of the line.
    fn strip_path_qualifier(s: &str) -> &str {
        let mut rest = s;
        loop {
            let ident_len = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .count();
            if ident_len == 0 {
                break;
            }
            match rest[ident_len..].strip_prefix("::") {
                Some(after) => rest = after,
                None => break,
            }
        }
        rest
    }

    for line in all.lines() {
        let trimmed = line.trim_start();

        // The invocation must be the first non-whitespace token on its line —
        // a commented-out `// impl_spec_display!(...)` no longer satisfies
        // this. A leading crate-path qualifier (`crate::`, `broadcast_common::`)
        // is transparent to this check: it is still the first *statement*.
        if let Some(rest) = strip_path_qualifier(trimmed).strip_prefix(&needle) {
            if is_boundary(rest) {
                return true;
            }
        }

        // A bare `Display for Name` needle (the generic fallback some crates
        // use) also counts when reached from the `impl` keyword through
        // nothing but a module-path qualifier: `impl ::core::fmt::Display for
        // Name`, `impl std::fmt::Display for Name`, `impl fmt::Display for
        // Name`, `impl Display for Name`.
        if !needle.starts_with("impl") {
            if let Some(after_impl) = trimmed.strip_prefix("impl") {
                if let Some(pos) = after_impl.find(&needle) {
                    let qualifier = &after_impl[..pos];
                    if qualifier.chars().all(is_path_or_space) {
                        let rest = &after_impl[pos + needle.len()..];
                        if is_boundary(rest) {
                            return true;
                        }
                    }
                }
            }
        }
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
