//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list, and fails if any remaining enum has neither
//! `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
//! impl. A present `Display` delegates to an inherent `name()`, so this one
//! check transitively enforces the whole convention (issue #806).
//!
//! This guard checks **per file**, not over the whole crate concatenated.
//! `StreamStatus` is declared independently by both `source::srt` and
//! `source::ts_http` (it is in SKIP below, so it's a latent risk rather than a
//! live gap, but the same hazard applies to any future same-named pair) — a
//! whole-crate scan would let one module's `impl_spec_display!` satisfy the
//! check for both same-named enums, so dropping either impl would go
//! unnoticed. `impl_spec_display!(Name)` resolves `Name` in the module it is
//! invoked from, so per-file is also the semantically correct scope.
//!
//! Skip list:
//! - `MultimuxError` (`error`) — structured `thiserror` error, not a spec/
//!   field label.
//! - `FileReaderError` (`source::file_reader`) — structured `thiserror` error
//!   for the file-reader task's read/probe/demux failures; not a spec/field
//!   label.
//! - `RtmpPushError` (`push::rtmp`) — thiserror error for the RTMP push
//!   transport's connect/protocol/IO failures; not a spec/field label.
//! - `RtspPushError` (`push::rtsp`) — thiserror error for the RTSP push
//!   transport's connect/protocol/IO failures; not a spec/field label.
//! - `SendMediaError` (`push`) — thiserror error for
//!   `PushTransport::send_media` (issue #934): mux failure vs. transport
//!   send failure; not a spec/field label.
//! - `InputSpec`, `AuthSpec`, `OutputAuthSpec` (`config`) — data-carrying
//!   config ADTs (`Rtsp { url, .. }`, `Password { .. }`, `Basic { .. }`, …);
//!   callers match the typed variant, a static label would be lossy.
//! - `DashResourceId`, `DashAction` (`source::dash_pull`) — data-carrying
//!   dispatch/identity ADTs (`Init(RepIndex)`, `FetchSegment { .. }`), same
//!   reasoning as `InputSpec`.
//! - `HlsFetchId` (`source::hls_pull`) — data-carrying dispatch ADT
//!   (`Resource(ResourceId)`), same reasoning.
//! - `SmoothResourceId`, `SmoothAction` (`source::smooth_pull`) — data-carrying
//!   dispatch/identity ADTs (`Fragment(StreamIdx, u64)`, `FetchManifest { .. }`),
//!   same reasoning.
//! - `StreamStatus` (`source::srt`, `source::ts_http`) — an internal
//!   read-loop outcome discriminant (`Fed`/`Ended`), not itself a wire-format
//!   field.
//! - `ReconnectState` (`push`) — the push reconnect FSM's state discriminant
//!   (`Ready`/`Backoff`/`Failed`), an internal driver enum whose three states
//!   carry no wire token; a static label would add nothing.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SKIP: &[&str] = &[
    "MultimuxError",
    "FileReaderError",
    "RtmpPushError",
    "RtspPushError",
    "SendMediaError",
    "InputSpec",
    "AuthSpec",
    "OutputAuthSpec",
    "DashResourceId",
    "DashAction",
    "HlsFetchId",
    "SmoothResourceId",
    "SmoothAction",
    "StreamStatus",
    "ReconnectState",
];

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

/// True if `{prefix}{name}` appears with an identifier boundary (whole name).
fn has_impl(src: &str, prefix: &str, name: &str) -> bool {
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

    for line in src.lines() {
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
            let labelled = has_impl(src, "impl_spec_display!(", &e)
                || has_impl(src, "impl ::core::fmt::Display for ", &e)
                || has_impl(src, "impl core::fmt::Display for ", &e)
                || has_impl(src, "impl std::fmt::Display for ", &e)
                || has_impl(src, "impl fmt::Display for ", &e);
            if !labelled {
                missing.push(format!("{e} (in {path})"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "public enum(s) missing a #204 Display/name() label (or a SKIP-list entry): {missing:?}"
    );
}
