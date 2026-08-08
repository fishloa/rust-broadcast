//! Drift-guard for the spec/field-enum label convention (issue #204).
//!
//! Scans this crate's `src/` for every `pub enum`, subtracts a documented
//! skip-list, and fails if any remaining enum has neither
//! `broadcast_common::impl_spec_display!(Name)` nor a hand-written `Display`
//! impl.
//!
//! # Why this crate's skip-list is almost everything
//!
//! The #204 convention exists for enums that decode an *externally-cited
//! spec's* coded field (a `table_id`, a `tag`, a `stream_type`) into a
//! stable, spec-grounded token. `media-plane` is not a wire-decode crate —
//! it is the ingress/byte-layer/Trunk/egress integration layer sitting
//! *above* the demuxers (`transmux`, `dvb-si`, …) that already do that
//! decoding. Its public enums are this crate's own control-flow, lifecycle,
//! and outcome types: `thiserror` errors, data-carrying ADTs (a poll result,
//! a driven-session event, a health state — each variant carries its own
//! payload, so a static label would be lossy and callers already match the
//! typed variant instead), and a handful of bare status/policy enums that
//! are this crate's *own* invented vocabulary, not a token from a cited
//! spec. [`crate::egress::CachePolicy`] is the one enum in this crate that
//! *does* carry the label pair (it doubles as an HTTP `Cache-Control` /
//! metrics token) and is deliberately not in this list.
//!
//! Because the project-wide `Display` impl delegates to an inherent
//! `name() -> &'static str`, a present `Display` transitively guarantees
//! `name()` exists (it would not compile otherwise) — so this single
//! coverage check enforces the whole convention and catches the one thing
//! the compiler cannot: a brand-new `pub enum` that nobody labelled or
//! triaged into this skip-list.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Enums that are intentionally **not** spec/field labels — see the module
/// doc for the two categories ("error" / "data-carrying ADT") and the third,
/// crate-specific one ("bare internal status/policy enum with no cited-spec
/// token").
const SKIP: &[&str] = &[
    // --- byte_merge.rs ---
    // Behaviour policy ADT: `Failover` carries `primary`/`secondary`/
    // `silence_timeout` fields, so a static label would drop them. Not a
    // spec-cited token — this crate's own merge-policy vocabulary.
    "MergePolicy",
    // Structured `thiserror` error enum — no spec label.
    "MergeError",
    // --- byte_tap.rs ---
    // Bare enum, but purely descriptive metadata about *this crate's own*
    // pipeline position (wire vs. post-transform), not a token from any
    // cited spec.
    "TapPoint",
    // Data-carrying ADT: `Data(Bytes, Timestamp)` / `Lagged { skipped }`
    // carry a payload each.
    "TapItem",
    // --- egress.rs ---
    // (`CachePolicy` already has `name()` + `impl_spec_display!` — not
    // listed here.)
    // Data-carrying ADT, generic over the response body type `B`
    // (`Ready { body, cache }` / `Await { .. }` / `BadRequest { reason }`).
    "EgressResponse",
    // Data-carrying ADT, generic over the caller's error type `E`
    // (`Accepted(TrackSelection)` / `Refused { reason }` / `Error(E)`).
    "NegotiationOutcome",
    // --- ingress.rs ---
    // Data-carrying ADT (`NewProgram { program, tracks }` / `Sample { .. }`)
    // — see the type's own doc for why it deliberately carries no label.
    "SessionEvent",
    // Data-carrying ADT, generic over the session's error type `E`
    // (`Failed(E)` / `HandshakeTimedOut { deadline }`).
    "HealthState",
    // Data-carrying ADT, generic over both the session type `S` and error
    // type `E` (a dial attempt's outcome, carrying the session or error).
    "DialAttempt",
    // Data-carrying ADT, generic over the listener's error type `E`
    // (`Admitted(SessionId)` / `Error(E)`).
    "AcceptOutcome",
    // --- retention.rs ---
    // Bare enum (`Taken`/`Busy`), but a sink hand-off status this crate
    // invented, not a token from any cited spec.
    "SinkOutcome",
    // Data-carrying ADT: `Tiered` carries a `cold_window`/sink
    // configuration, not a bare label.
    "Retention",
    // Bare enum (`Hot`/`Cold`/`Evicted`), but this crate's own
    // catch-up-locate status, not a cited-spec token.
    "SegmentLocation",
    // --- trunk.rs ---
    // Bare enum (`Timed`/`Sparse`), but this crate's own retention-ring
    // classification, not a cited-spec token.
    "RetentionClass",
    // Bare enum (`Gap`/`StallIngest`/`Terminate`), but a caller-chosen
    // trade this crate's own DVR design invented, not a cited-spec token.
    "ArchiveOverrun",
    // Data-carrying ADT (the event log's absolute/segment-relative anchor
    // choice, each variant carrying its own timestamp/segment data).
    "EventAnchor",
    // Data-carrying ADT (`Timed { track_id, sample }` /
    // `Sparse { .. }` / `Lagged { skipped }`).
    "SampleCursorItem",
    // Data-carrying ADT (a finished segment's fields, or `Gap`/`Terminated`
    // with their own data).
    "SegmentCursorItem",
    // Data-carrying ADT (a timed-metadata event plus its anchor, or a
    // `Lagged`/gap variant with its own data).
    "EventCursorItem",
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
