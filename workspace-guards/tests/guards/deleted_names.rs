//! Names a finished refactor deleted may not reappear anywhere in a crate's
//! `src/`, `tests/` or `examples/`.
//!
//! The streaming-first programme replaced every whole-`Media` muxer, transform
//! and protection type with a stage, and the Phase 3 slice 6 removed the
//! whole-input MP4 demuxers for `transmux::mp4_demux::Mp4Demux`. A test,
//! example or doc comment that names a deleted type is either stale or a
//! partial revert; both are caught here. This is a lexical tripwire, not a
//! proof: a name built at runtime or spelled in a macro evades it, so code
//! review remains the real control.
//!
//! Each table entry is reached from the identifier before its last `::`, so
//! the 150-odd names cost one scan per distinct needle (`transmux`,
//! `broadcast_common`, …) rather than one scan per name, while every file is
//! still read exactly once.

use workspace_guards::{crate_dir, rs_files, workspace_members};

use crate::deleted_names_data::{EXEMPT, TABLE};

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `true` when the character at byte `end` does not continue an identifier.
fn ends_identifier(text: &str, end: usize) -> bool {
    text[end..].chars().next().is_none_or(|c| !is_ident(c))
}

/// Byte offsets of every occurrence of `needle` in `text` that starts a whole
/// identifier.
fn identifier_starts<'t>(text: &'t str, needle: &'t str) -> impl Iterator<Item = usize> + 't {
    text.match_indices(needle)
        .map(|(begin, _)| begin)
        .filter(move |&begin| {
            text[..begin]
                .chars()
                .next_back()
                .is_none_or(|c| !is_ident(c))
        })
}

/// `true` when `name` occurs in `text` as a whole identifier rather than as
/// the prefix of a longer one (`CmafMux` must not match `CmafMuxStage`, and
/// `broadcast_common::wire` must not match `broadcast_common::wire_enum`).
fn names_whole_identifier(text: &str, name: &str) -> bool {
    identifier_starts(text, name).any(|begin| ends_identifier(text, begin + name.len()))
}

fn matching_brace(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' if depth == 0 => return Some(i),
            '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The identifier a table entry is reached from: the name itself when it is
/// unqualified, otherwise everything before its final `::`.
fn needle_of(name: &str) -> &str {
    name.rsplit_once("::").map_or(name, |(prefix, _)| prefix)
}

/// `true` when an occurrence of [`needle_of`]`(name)` beginning at `begin`
/// spells `name`: as written (`prefix::leaf`), or as a member of a
/// `prefix::{...}` group.
fn match_at(text: &str, begin: usize, name: &str) -> bool {
    let Some((prefix, leaf)) = name.rsplit_once("::") else {
        return ends_identifier(text, begin + name.len());
    };
    let rest = &text[begin + prefix.len()..];
    if let Some(after) = rest.strip_prefix("::")
        && after.starts_with(leaf)
        && ends_identifier(after, leaf.len())
    {
        return true;
    }
    let trimmed = rest.trim_start();
    if let Some(after) = trimmed.strip_prefix("::") {
        let after = after.trim_start();
        if let Some(after) = after.strip_prefix('{')
            && let Some(end) = matching_brace(after)
            && names_whole_identifier(&after[..end], leaf)
        {
            return true;
        }
    }
    false
}

/// `true` when `name` is spelled in `text` as a whole identifier.
fn names_deleted(text: &str, name: &str) -> bool {
    identifier_starts(text, needle_of(name)).any(|begin| match_at(text, begin, name))
}

/// The table grouped by [`needle_of`], so each needle is scanned for once.
struct Group {
    needle: &'static str,
    /// Table indices, in table order.
    entries: Vec<usize>,
}

fn groups() -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for (index, banned) in TABLE.iter().enumerate() {
        let needle = needle_of(banned.name);
        match out.iter_mut().find(|group| group.needle == needle) {
            Some(group) => group.entries.push(index),
            None => out.push(Group {
                needle,
                entries: vec![index],
            }),
        }
    }
    out
}

fn scan_file(path: &std::path::Path, text: &str, groups: &[Group], problems: &mut Vec<String>) {
    let mut hits: Vec<(usize, usize)> = Vec::new();
    for group in groups {
        for begin in identifier_starts(text, group.needle) {
            for &index in &group.entries {
                if match_at(text, begin, TABLE[index].name) {
                    hits.push((index, begin));
                }
            }
        }
    }
    hits.sort_unstable();
    let mut previous: Option<(usize, usize)> = None;
    for (index, offset) in hits {
        let line = text[..offset].matches('\n').count() + 1;
        if previous == Some((index, line)) {
            continue;
        }
        previous = Some((index, line));
        let banned = &TABLE[index];
        problems.push(format!(
            "{}:{}: `{}` is gone ({}): {}",
            path.display(),
            line,
            banned.name,
            banned.issue,
            banned.reason
        ));
    }
}

#[test]
fn deleted_names_stay_deleted() {
    let groups = groups();
    let mut problems = Vec::new();
    for member in workspace_members() {
        if EXEMPT.contains(&member.as_str()) {
            continue;
        }
        let dir = crate_dir(&member);
        for area in ["src", "tests", "examples"] {
            let root = dir.join(area);
            if !root.is_dir() {
                continue;
            }
            for file in rs_files(&root) {
                let Ok(text) = std::fs::read_to_string(&file) else {
                    continue;
                };
                scan_file(&file, &text, &groups, &mut problems);
            }
        }
    }
    assert!(
        problems.is_empty(),
        "a deleted name reappeared (see workspace-guards/tests/guards/deleted_names_data.rs):\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_table_is_not_empty() {
    assert!(
        !TABLE.is_empty(),
        "the deleted-name table must name at least one symbol"
    );
}

#[test]
fn a_replacement_name_does_not_count_as_the_deleted_one() {
    assert!(!names_deleted("CmafMuxStage", "CmafMux"));
    assert!(!names_deleted("TsMuxStage::new", "TsMux"));
    assert!(names_deleted("the CmafMux stage", "CmafMux"));
    assert!(names_deleted("`TsMux`", "TsMux"));
    assert!(names_deleted("crate::splice::concat", "splice::concat"));
}

#[test]
fn a_grouped_import_names_every_member() {
    assert!(names_deleted(
        "use broadcast_common::{Parse, Serialize};",
        "broadcast_common::Parse"
    ));
    assert!(names_deleted(
        "use broadcast_common::{bcd, clock33, Serialize};",
        "broadcast_common::Serialize"
    ));
    assert!(names_deleted(
        "//! use broadcast_common::{Parse, Serialize};",
        "broadcast_common::Parse"
    ));
    assert!(names_deleted(
        "use broadcast_common::{len::{need_in}, Parse};",
        "broadcast_common::len"
    ));
    assert!(names_deleted(
        "use broadcast_common::{\n    Parse,\n    Serialize,\n};",
        "broadcast_common::Serialize"
    ));
    assert!(!names_deleted(
        "use broadcast_core::{Parse, Serialize};",
        "broadcast_common::Parse"
    ));
    assert!(!names_deleted(
        "use broadcast_common::{bcd, clock33, crc32_mpeg2};",
        "broadcast_common::Parse"
    ));
}

#[test]
fn a_qualified_name_stops_at_the_identifier_boundary() {
    assert!(names_deleted(
        "use broadcast_common::wire;",
        "broadcast_common::wire"
    ));
    assert!(names_deleted(
        "broadcast_common::wire::read_u8(bytes)",
        "broadcast_common::wire"
    ));
    assert!(names_deleted(
        "use broadcast_common::{clock33, wire};",
        "broadcast_common::wire"
    ));
    assert!(!names_deleted(
        "broadcast_common::wire_enum! { Apple = 1 }",
        "broadcast_common::wire"
    ));
    assert!(!names_deleted(
        "use broadcast_common::wire_enum;",
        "broadcast_common::wire"
    ));
}
