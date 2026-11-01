//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! For every table crate: scan `src/` for every `pub enum`, subtract the crate's
//! `skip` list, and fail if any remaining enum has neither
//! `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
//! impl. A present `Display` delegates to an inherent `name()`, so this one
//! check transitively enforces the whole convention (issue #806).
//!
//! Three matcher variants the per-crate copies already used are kept per crate:
//! `Strict` (macro or one of five exact `impl .. Display for` prefixes),
//! `Loose` (macro or any `impl <path> Display for`), and `Dash` (dash-runtime's
//! own spelling). `Scope::PerFile` requires the impl to sit in the same file as
//! the enum; `Scope::Crate` accepts it anywhere in the crate. `expected` pins
//! the exact set of non-skipped spec/field enums so a new one cannot appear
//! unnoticed; `min_enums` guards against a scanner that silently finds nothing.

use crate::common::assert_table_covers_members;
use std::collections::BTreeSet;
use workspace_guards::src_files;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Crate,
    PerFile,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Matcher {
    Strict,
    Loose,
    Dash,
}

pub struct Crate {
    pub name: &'static str,
    pub scope: Scope,
    pub matcher: Matcher,
    pub skip: &'static [&'static str],
    pub expected: Option<&'static [&'static str]>,
    pub min_enums: Option<usize>,
}

/// True if `{prefix}{name}` appears with an identifier boundary (whole name).
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

fn has_display(all: &str, name: &str) -> bool {
    let macro_needle = format!("impl_spec_display!({name}");
    let boundary =
        |rest: &str| !matches!(rest.chars().next(), Some(c) if c.is_alphanumeric() || c == '_');
    all.lines().any(|line| {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&macro_needle)
            && boundary(rest)
        {
            return true;
        }
        t.strip_prefix("impl")
            .and_then(|after| {
                after
                    .find(&format!("Display for {name}"))
                    .map(|p| (after, p))
            })
            .is_some_and(|(after, p)| {
                let rest = &after[p + format!("Display for {name}").len()..];
                boundary(rest)
            })
    })
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

fn labelled(matcher: Matcher, src: &str, name: &str) -> bool {
    match matcher {
        Matcher::Strict => {
            has_impl(src, "impl_spec_display!(", name)
                || has_impl(src, "impl ::core::fmt::Display for ", name)
                || has_impl(src, "impl core::fmt::Display for ", name)
                || has_impl(src, "impl std::fmt::Display for ", name)
                || has_impl(src, "impl fmt::Display for ", name)
        }
        Matcher::Loose => {
            has_impl(src, "impl_spec_display!(", name) || has_impl(src, "Display for ", name)
        }
        Matcher::Dash => has_display(src, name),
    }
}

/// `"crate: enum (in path)"` for each non-skipped enum without a label, and
/// the set of non-skipped enum names the crate declares.
fn scan(c: &Crate, files: &[(String, String)]) -> (Vec<String>, BTreeSet<String>) {
    let mut missing = Vec::new();
    let mut non_skip = BTreeSet::new();
    match c.scope {
        Scope::PerFile => {
            for (path, src) in files {
                for e in pub_enums(src) {
                    if c.skip.contains(&e.as_str()) {
                        continue;
                    }
                    non_skip.insert(e.clone());
                    if !labelled(c.matcher, src, &e) {
                        missing.push(format!("{}: {e} (in {path})", c.name));
                    }
                }
            }
        }
        Scope::Crate => {
            let all = files
                .iter()
                .map(|(_, s)| s.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let enums = pub_enums(&all);
            if let Some(min) = c.min_enums {
                assert!(
                    enums.len() >= min,
                    "{}: scanner found only {enums:?}",
                    c.name
                );
            }
            for e in &enums {
                if c.skip.contains(&e.as_str()) {
                    continue;
                }
                non_skip.insert(e.clone());
                if !labelled(c.matcher, &all, e) {
                    let at = files
                        .iter()
                        .find(|(_, s)| pub_enums(s).contains(e))
                        .map_or("?", |(p, _)| p.as_str());
                    missing.push(format!("{}: {e} (declared in {at})", c.name));
                }
            }
        }
    }
    (missing, non_skip)
}

#[test]
fn every_public_spec_enum_has_a_display_impl() {
    let mut missing = Vec::new();
    for c in crate::label_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        missing.extend(scan(c, &files).0);
    }
    assert!(
        missing.is_empty(),
        "public enum(s) missing a #204 Display/name() label (or a skip entry in label_data.rs):\n{}\n\
         Add `broadcast_common::impl_spec_display!(Name)` plus an inherent `name()`, \
         or add the enum to the crate's skip list if it is not a spec/field label.",
        missing.join("\n")
    );
}

#[test]
fn expected_spec_enum_sets_have_not_silently_drifted() {
    for c in crate::label_data::TABLE {
        let Some(expected) = c.expected else {
            continue;
        };
        let (_, non_skip) = scan(c, &src_files(c.name));
        let expected: BTreeSet<String> = expected.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            non_skip, expected,
            "{}: the set of public spec/field enums changed; label the new enum (or add it to \
             skip with a reason) and update `expected` in label_data.rs",
            c.name
        );
    }
}

#[test]
fn every_error_kind_variant_has_a_stable_name() {
    let src = workspace_guards::crate_dir("broadcast-client").join("src/events.rs");
    let text = std::fs::read_to_string(src).expect("read events.rs");
    let body_start = text
        .find("pub enum ErrorKind {")
        .expect("ErrorKind declaration");
    let body = &text[body_start..];
    let end = body.find("\n}\n").expect("end of ErrorKind");
    let mut variants = BTreeSet::new();
    for line in body[..end].lines().skip(1) {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('#') || t.is_empty() {
            continue;
        }
        if line.starts_with("    ") && !line.starts_with("     ") {
            let name: String = t
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                variants.insert(name);
            }
        }
    }
    assert!(variants.len() >= 19, "found {variants:?}");
    let name_fn = &text[text.find("pub fn name(&self) -> &'static str {\n        match self {\n            Self::UnsupportedEncryption").expect("ErrorKind::name")..];
    let name_fn = &name_fn[..name_fn.find("\n    }\n").unwrap()];
    for v in &variants {
        assert!(
            name_fn.contains(&format!("Self::{v} ")) || name_fn.contains(&format!("Self::{v},")),
            "ErrorKind::{v} has no arm in name()"
        );
    }
}

#[test]
fn table_covers_every_workspace_member() {
    let names: Vec<&str> = crate::label_data::TABLE.iter().map(|c| c.name).collect();
    assert_table_covers_members("label", &names, crate::label_data::EXEMPT);
}

#[test]
fn detector_bites() {
    let c = |scope, matcher| Crate {
        name: "synthetic",
        scope,
        matcher,
        skip: &["Error"],
        expected: None,
        min_enums: None,
    };
    let files = vec![
        (
            "src/a.rs".to_string(),
            "pub enum Good { A }\nimpl_spec_display!(Good);\npub enum Bad { A }\npub enum Error { A }\n"
                .to_string(),
        ),
        (
            "src/b.rs".to_string(),
            "impl core::fmt::Display for Bad {}\n".to_string(),
        ),
    ];
    let (m, _) = scan(&c(Scope::Crate, Matcher::Strict), &files);
    assert!(m.is_empty(), "{m:?}");
    let (m, _) = scan(&c(Scope::PerFile, Matcher::Strict), &files);
    assert_eq!(m, vec!["synthetic: Bad (in src/a.rs)".to_string()]);
    assert!(!labelled(
        Matcher::Strict,
        "// impl_spec_display!(Good);\n",
        "Good"
    ));
}
