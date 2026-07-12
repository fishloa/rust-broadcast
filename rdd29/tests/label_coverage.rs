//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list, and fails if any remaining enum has neither
//! `broadcast_common::impl_spec_display!(Name)` nor a hand-written
//! `Display` impl.
//!
//! `rdd29` has eight spec/field enums, each carrying `name()` +
//! `impl_spec_display!(...)`: [`rdd29::ElementId`] (Table 1),
//! [`rdd29::SampleRate`] (Table 2), [`rdd29::BitDepth`] (Table 3),
//! [`rdd29::FrameRate`] (Table 4), [`rdd29::ChannelId`] (Table 6),
//! [`rdd29::ZoneGain`] (Table 9), [`rdd29::ZoneId`] (Table 8 — an index
//! label, not itself a wire field, but still spec-cited and labelled),
//! [`rdd29::ObjectSpreadMode`] (Table 10), and [`rdd29::DecorCoefPrefix`]
//! (Table 11). [`rdd29::AnyElement`] is a tag-dispatch enum (like
//! `dvb_si::AnyDescriptor`), not a spec/field label, and is skipped.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Enums that are intentionally **not** spec/field labels: the structured
/// error type has no spec label, and `AnyElement` is a tag-dispatch enum
/// over the three concrete element types (see this crate's module docs).
const SKIP: &[&str] = &["Error", "AnyElement"];

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

fn all_pub_enums(all: &str) -> BTreeSet<String> {
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
    enums
}

fn crate_src() -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    read_rs(&src, &mut files);
    files.join("\n")
}

#[test]
fn every_public_spec_enum_has_a_display_impl() {
    let all = crate_src();
    let enums = all_pub_enums(&all);

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
fn expected_spec_enum_set_has_not_silently_drifted() {
    // A cheap sanity check that the crate's enum surface hasn't grown (or
    // shrunk) without anyone updating this file's doc comment.
    let all = crate_src();
    let enums = all_pub_enums(&all);
    let non_skip: Vec<_> = enums
        .iter()
        .filter(|e| !SKIP.contains(&e.as_str()))
        .cloned()
        .collect();

    assert_eq!(
        non_skip,
        vec![
            "BitDepth",
            "ChannelId",
            "DecorCoefPrefix",
            "ElementId",
            "FrameRate",
            "ObjectSpreadMode",
            "SampleRate",
            "ZoneGain",
            "ZoneId",
        ],
        "the set of public spec/field enums changed; if you added a new one, label it \
         (see this file's doc comment) and update this assertion"
    );
}
