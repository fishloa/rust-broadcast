//! Drift-guard for the spec/field-enum label convention (issue #204), local
//! variant.
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list, and fails if any remaining enum has no `Display` impl. A present
//! `Display` here delegates to an inherent `name() -> &'static str` (it would
//! not compile otherwise), so this single check enforces the whole convention.
//!
//! `rtsp-runtime` is a standalone RTSP crate with no DVB dependency, so it does
//! **not** pull `broadcast_common::impl_spec_display!`; the spec/field enums
//! (`SessionState`, `LowerTransport`, `Delivery`) carry hand-written `name()` +
//! `Display` instead. This guard accepts either form.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Enums that are intentionally **not** spec/field labels: the structured error
/// and the two data-carrying event ADTs whose variants hold payloads (a static
/// label would be lossy — callers match the typed variant instead).
const SKIP: &[&str] = &[
    "Error",       // structured thiserror error
    "ClientEvent", // data-carrying ADT (Response/AuthRetry/MediaData)
    "ServerEvent", // data-carrying ADT (RequestAccepted/MethodNotValid/SessionSetup)
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

    let missing: Vec<_> = enums
        .iter()
        .filter(|e| !SKIP.contains(&e.as_str()))
        .filter(|e| !has_impl(&all, "Display for ", e))
        .cloned()
        .collect();

    assert!(
        missing.is_empty(),
        "pub enum(s) missing a Display impl (issue #204 convention): {missing:?}\n\
         Add a hand-written `name()` + `Display`, or add the enum to SKIP if it \
         is not a spec/field label."
    );
}
