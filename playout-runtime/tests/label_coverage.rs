//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! `Error` is the crate's error enum (data-carrying variants, not a spec
//! label) — SKIPped, same convention `ssai-runtime`'s own
//! `tests/label_coverage.rs` documents for its `Error`.
//!
//! `EntryKind` (`schedule.rs`) and `BreakEdge` (`scte35.rs`) ARE this
//! crate's own field/token enums (no external wire spec backs them — issue
//! #748's design decision is that the schedule format is ours to define —
//! but the #204 convention applies uniformly to every public token enum
//! regardless of whether an external spec assigns the token) and carry
//! `name()` + `impl_spec_display!` like any other.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SKIP: &[&str] = &["Error"];

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
/// char (so `Method` does not match `MethodKind`) — tolerating a leading
/// module-path qualifier (`broadcast_common::impl_spec_display!(...)`),
/// since every invocation in this crate is fully qualified rather than
/// `use`-imported.
// Hardened variant, copied verbatim from `transmux/tests/label_coverage.rs`.
// The earlier version used a bare `line.find(&needle)`, so a commented-out
// `// impl_spec_display!(X);` satisfied the guard — demonstrated, not
// theorised: commenting out `impl_spec_display!(SnapMode)` left this test
// green. The rest of the workspace was already hardened; these two newest
// crates were copied from a stale template.
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
        if let Some(rest) = strip_path_qualifier(trimmed).strip_prefix(&needle)
            && is_boundary(rest)
        {
            return true;
        }

        // A bare `Display for Name` needle (the generic fallback some crates
        // use) also counts when reached from the `impl` keyword through
        // nothing but a module-path qualifier: `impl ::core::fmt::Display for
        // Name`, `impl std::fmt::Display for Name`, `impl fmt::Display for
        // Name`, `impl Display for Name`.
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

    let expected: BTreeSet<String> = ["BreakEdge", "EntryKind"]
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
