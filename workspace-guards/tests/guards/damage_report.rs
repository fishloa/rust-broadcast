//! Guard: a reader that tolerates damage reports it one way.
//!
//! `broadcast_core::damage` owns the one report of content a reader skipped
//! (`Damage`), and `docs/API-CONVENTIONS.md` makes a silent skip a defect. This
//! family fails when a reader crate in its table
//!
//! - declares a public enum that is a report of skipped content (its name
//!   carries one of [`REPORT_NAME_NEEDLES`]) but does not list it in the
//!   table, so a new private vocabulary cannot appear unseen; or
//! - lists a report enum that neither carries a `Damage` (a variant holding
//!   one) nor converts to one (an `impl` block naming the enum whose body
//!   mentions `Damage`).
//!
//! The scan is lexical, not a proof: a renamed enum or a helper declared in a
//! macro evades it, so review is the real control.

use crate::common::assert_names_are_members;
use crate::damage_report_data::{READERS, REPORT_NAME_NEEDLES, Reader};
use std::collections::BTreeSet;
use workspace_guards::src_files;

/// The body of `pub enum <name>` (`{`..matching `}`), or `None`.
fn enum_body(text: &str, name: &str) -> Option<String> {
    let needle = format!("pub enum {name}");
    let start = text.find(&needle)?;
    brace_block(&text[start..])
}

/// The braces-delimited block starting at the first `{` of `text`.
fn brace_block(text: &str) -> Option<String> {
    let open = text.find('{')?;
    let mut depth = 0usize;
    for (idx, ch) in text[open..].char_indices() {
        match ch {
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(text[..open + idx + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Every `impl` header line plus its brace-delimited body.
fn impl_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find("impl") {
        let after = &rest[pos..];
        let boundary = after[4..]
            .chars()
            .next()
            .is_some_and(|c| c == '<' || c == ' ' || c.is_alphanumeric() || c == '_');
        if !boundary {
            rest = &after[4..];
            continue;
        }
        if let Some(block) = brace_block(after) {
            out.push(block);
        }
        rest = &after[4..];
    }
    out
}

/// Whether the crate's sources carry a `Damage` in `name`, or convert `name`
/// into one.
fn carries_or_converts(files: &[(String, String)], name: &str) -> bool {
    let carried = files
        .iter()
        .any(|(_, text)| enum_body(text, name).is_some_and(|body| body.contains("Damage")));
    if carried {
        return true;
    }
    files.iter().any(|(_, text)| {
        impl_blocks(text)
            .into_iter()
            .any(|block| header_names(&block, name) && block.contains("Damage"))
    })
}

/// Whether an `impl` block's header (the text before its first `{`) names
/// `name` as a whole identifier.
fn header_names(block: &str, name: &str) -> bool {
    let header = block.split('{').next().unwrap_or("");
    let mut search = header;
    while let Some(pos) = search.find(name) {
        let before = search[..pos].chars().next_back();
        let after = search[pos + name.len()..].chars().next();
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !ident(before) && !ident(after) {
            return true;
        }
        search = &search[pos + name.len()..];
    }
    false
}

/// Problems for one reader crate, from its `(path, text)` sources.
fn problems(reader: &Reader, files: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    let declared: BTreeSet<String> = files
        .iter()
        .flat_map(|(_, text)| {
            text.match_indices("pub enum ")
                .filter_map(|(pos, needle)| {
                    let rest = &text[pos + needle.len()..];
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    (!name.is_empty()).then_some(name)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for name in &declared {
        if REPORT_NAME_NEEDLES
            .iter()
            .any(|needle| name.contains(needle))
            && !reader.reports.contains(&name.as_str())
        {
            out.push(format!(
                "{}: public enum `{name}` reports skipped content but is not listed in the \
                 damage_report table; carry a broadcast_core::damage::Damage in a variant (or \
                 convert to one) and list it",
                reader.name
            ));
        }
    }
    for name in reader.reports {
        if !declared.contains(*name) {
            out.push(format!(
                "{}: the damage_report table lists `{name}`, which the crate no longer declares",
                reader.name
            ));
            continue;
        }
        if !carries_or_converts(files, name) {
            out.push(format!(
                "{}: `{name}` neither carries a broadcast_core::damage::Damage in a variant nor \
                 converts to one in an impl block",
                reader.name
            ));
        }
    }
    out
}

#[test]
fn readers_report_through_damage() {
    assert_names_are_members(
        "damage_report",
        &READERS.iter().map(|r| r.name).collect::<Vec<_>>(),
    );
    let mut all = Vec::new();
    for reader in READERS {
        all.extend(problems(reader, &src_files(reader.name)));
    }
    assert!(all.is_empty(), "{}", all.join("\n"));
}

#[test]
fn a_planted_finding_enum_in_a_converted_crate_trips_the_guard() {
    let reader = READERS.first().expect("a converted reader");
    let planted = vec![(
        "src/planted.rs".to_string(),
        "pub enum PacketFinding { Skipped, Repaired }".to_string(),
    )];
    let found = problems(reader, &planted);
    assert!(
        found.iter().any(|p| p.contains("PacketFinding")),
        "a planted report enum must trip the guard: {found:?}"
    );
    assert!(
        found.iter().any(|p| p.contains("no longer declares")),
        "the planted source lacks the crate's listed reports: {found:?}"
    );
}

#[test]
fn a_listed_report_enum_without_a_damage_trips_the_guard() {
    let reader = READERS.first().expect("a converted reader");
    let files: Vec<(String, String)> = reader
        .reports
        .iter()
        .map(|name| {
            (
                "src/planted.rs".to_string(),
                format!("pub enum {name} {{ Nothing }}"),
            )
        })
        .collect();
    let found = problems(reader, &files);
    for name in reader.reports {
        assert!(
            found
                .iter()
                .any(|p| p.contains(name) && p.contains("neither carries")),
            "`{name}` without a Damage must trip the guard: {found:?}"
        );
    }
}

#[test]
fn a_planted_finding_enum_in_the_isobmff_reader_trips_the_guard() {
    let reader = READERS
        .iter()
        .find(|r| r.name == "isobmff")
        .expect("the isobmff reader");
    let planted = vec![(
        "src/planted.rs".to_string(),
        "pub enum BoxFinding { Skipped, Repaired }".to_string(),
    )];
    let found = problems(reader, &planted);
    assert!(
        found.iter().any(|p| p.contains("BoxFinding")),
        "a planted private finding enum must trip the guard: {found:?}"
    );
}

#[test]
fn a_conversion_satisfies_the_guard() {
    let reader = READERS.first().expect("a converted reader");
    let files: Vec<(String, String)> = reader
        .reports
        .iter()
        .map(|name| {
            (
                "src/planted.rs".to_string(),
                format!(
                    "pub enum {name} {{ Nothing }}\n\
                     impl From<{name}> for broadcast_core::damage::Damage {{\n\
                     fn from(_: {name}) -> Self {{ todo!() }}\n}}\n"
                ),
            )
        })
        .collect();
    assert!(problems(reader, &files).is_empty());
}
