//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list (error enums that carry no spec label), and fails if any
//! remaining enum has neither `broadcast_common::impl_spec_display!(Name)`
//! nor a hand-written `Display` impl.
//!
//! `st377-1` defines five public spec/field enums today:
//! [`st377_1::PartitionKind`], [`st377_1::PartitionStatus`] (Table 4),
//! [`st377_1::ItemLengthMode`] (§9.3 Note 1), [`st377_1::ReleaseType`]
//! (§4.3's `ProductVersion.release` enumeration), and
//! [`st377_1::StructuralSetKind`] (Table 17 — hand-written `Display` since
//! its `Unknown` catch-all carries a 2-byte payload the
//! `impl_spec_display!` macro's single-byte-payload form doesn't fit).

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
         Add `broadcast_common::impl_spec_display!(Name)` (or a hand-written \
         `impl Display for Name`), or add the enum to SKIP if it is not a \
         spec/field label."
    );
}
