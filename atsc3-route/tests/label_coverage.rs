//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! `atsc3-route` defines four public spec/field label enums — [`Codepoint`],
//! [`FormatId`], [`FragMode`] (all in `codepoint.rs`) and [`ExtTol`]
//! (`ext.rs`) — each carrying `name()` + `impl_spec_display!`.
//!
//! `Error` is skipped (structured error, no spec label). `RouteFecPayloadId`
//! (`fec.rs`) is also skipped: it is a **dispatch enum** selecting between
//! two disjoint wire layouts by an out-of-band bit (the packet's LCT SPI),
//! not itself a spec/field token — the same category as the workspace's
//! `Any*` tag-dispatch enums. See its module doc comment for the full
//! rationale.
//!
//! Checked **per file**, not over the whole crate concatenated, so a future
//! module reusing a variant/enum name cannot let one module's
//! `impl_spec_display!` silently satisfy the check for a different enum of
//! the same name declared elsewhere.
//!
//! [`Codepoint`]: atsc3_route::Codepoint
//! [`FormatId`]: atsc3_route::FormatId
//! [`FragMode`]: atsc3_route::FragMode
//! [`ExtTol`]: atsc3_route::ExtTol

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SKIP: &[&str] = &["Error", "RouteFecPayloadId"];

/// Every `.rs` file under `src/`, as `(display path, contents)`.
fn read_rs(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            read_rs(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            let text = fs::read_to_string(&path).expect("read .rs");
            out.push((path.display().to_string(), text));
        }
    }
}

/// True if `src` contains `{prefix}{name}` not followed by an identifier
/// char (so `Codepoint` does not match `CodepointSemantics`).
fn has_impl(src: &str, prefix: &str, name: &str) -> bool {
    let needle = format!("{prefix}{name}");
    let is_boundary =
        |rest: &str| !matches!(rest.chars().next(), Some(c) if c.is_alphanumeric() || c == '_');
    let is_path_or_space =
        |c: char| c.is_whitespace() || c == ':' || c.is_alphanumeric() || c == '_';

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

    for line in src.lines() {
        let trimmed = line.trim_start();

        if let Some(rest) = strip_path_qualifier(trimmed).strip_prefix(&needle)
            && is_boundary(rest)
        {
            return true;
        }

        if !needle.starts_with("impl")
            && let Some(after_impl) = trimmed.strip_prefix("impl")
            && let Some(pos) = after_impl.find(&needle)
        {
            let qualifier = &after_impl[..pos];
            let rest = &after_impl[pos + needle.len()..];
            if qualifier.chars().all(is_path_or_space) && is_boundary(rest) {
                return true;
            }
        }
    }

    false
}

fn pub_enums(src: &str) -> BTreeSet<String> {
    let mut enums = BTreeSet::new();
    for line in src.lines() {
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
    enums
}

fn crate_files() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    read_rs(&src, &mut files);
    files
}

#[test]
fn every_public_spec_enum_has_a_display_impl() {
    let mut missing: Vec<String> = Vec::new();

    for (path, src) in &crate_files() {
        for e in pub_enums(src) {
            if SKIP.contains(&e.as_str()) {
                continue;
            }
            if !has_impl(src, "impl_spec_display!(", &e) && !has_impl(src, "Display for ", &e) {
                missing.push(format!("{e} (in {path})"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "pub enum(s) missing a Display impl (issue #204 convention): {missing:?}\n\
         Add `broadcast_common::impl_spec_display!(Name)` plus an inherent `name()`, \
         or add the enum to SKIP if it is not a spec/field label."
    );
}

#[test]
fn expected_spec_enum_set_has_not_silently_drifted() {
    let non_skip: BTreeSet<String> = crate_files()
        .iter()
        .flat_map(|(_, src)| pub_enums(src))
        .filter(|e| !SKIP.contains(&e.as_str()))
        .collect();

    let expected: BTreeSet<String> = ["Codepoint", "FormatId", "FragMode", "ExtTol"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    assert_eq!(
        non_skip, expected,
        "the public spec/field enum set drifted.\n\
         Either add `name()` + `impl_spec_display!` and update this assertion, \
         or add the enum to SKIP."
    );
}
