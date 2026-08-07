//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list (error / data-carrying ADT / wire-variant enums that carry no
//! spec label), and fails if any remaining enum has neither
//! `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
//! impl.
//!
//! Because the project-wide `Display` impl delegates to an inherent
//! `name() -> &'static str`, a present `Display` transitively guarantees
//! `name()` exists (it would not compile otherwise) — so this single coverage
//! check enforces the whole convention and catches the one thing the compiler
//! cannot: a brand-new `pub enum` that nobody labelled.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Enums that are intentionally **not** spec/field labels.
const SKIP: &[&str] = &[
    // Structured `thiserror` error enum — no spec label.
    "RtmpError",
    // Data-carrying ADT (§8.2 AMF0 value types): each variant holds its own
    // payload (Number(f64), Utf8String<'a>, Object(..), ...), so a static
    // label would be lossy — callers match the typed variant instead. See
    // the doc comment on `Amf0Value` in amf0.rs for the full rationale.
    "Amf0Value",
    // Wire-variant ADT (§5.3.1.2 Chunk Message Header Type 0/1/2/3): each
    // variant holds a different set of resolved header fields, not a bare
    // spec label — `Fmt` (the 2-bit format selector) is the labelled
    // counterpart already carrying `name()`/`impl_spec_display!`.
    "MessageHeader",
    // Data-carrying event ADT `ServerSession::handle_data` surfaces to the
    // caller (Connected{app}/Publish{..}/Media{flv}/Eof) — not a spec label.
    "ServerEvent",
    // Data-carrying event ADT `ClientSession::handle_data` surfaces to the
    // caller (Connected/StreamCreated/Publishing/Error/Closed) — not a spec label.
    "ClientEvent",
    // Internal client state machine discriminant, not a spec label.
    "ClientState",
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
