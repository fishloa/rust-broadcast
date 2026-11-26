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

use workspace_guards::{crate_dir, rs_files, workspace_members};

use crate::deleted_names_data::{Banned, EXEMPT, TABLE};

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn names_whole_identifier(text: &str, name: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = text[from..].find(name) {
        let begin = from + offset;
        let end = begin + name.len();
        let starts = text[..begin]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident(c));
        let ends = text[end..].chars().next().is_none_or(|c| !is_ident(c));
        if starts && ends {
            return true;
        }
        from = end;
    }
    false
}

/// Byte offsets of every `prefix::{...}` group in `text` whose braces contain
/// `leaf` as a whole identifier.
fn group_offsets(text: &str, prefix: &str, leaf: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find(prefix) {
        let begin = from + found;
        let after = begin + prefix.len();
        if text[..begin]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident(c))
        {
            let rest = text[after..].trim_start();
            if let Some(rest) = rest.strip_prefix("::") {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('{')
                    && let Some(end) = matching_brace(rest)
                    && names_whole_identifier(&rest[..end], leaf)
                {
                    offsets.push(begin);
                }
            }
        }
        from = after;
    }
    offsets
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

/// `true` when `name` occurs in `text` as a whole identifier rather than as
/// the prefix of a longer one (`CmafMux` must not match `CmafMuxStage`). A
/// qualified name (one containing `::`) matches as written or as a member of
/// a `prefix::{...}` group.
fn names_deleted(text: &str, name: &str) -> bool {
    if let Some((prefix, leaf)) = name.rsplit_once("::") {
        return text.contains(name) || !group_offsets(text, prefix, leaf).is_empty();
    }
    names_whole_identifier(text, name)
}

fn scan_file(path: &std::path::Path, text: &str, banned: &Banned, problems: &mut Vec<String>) {
    let mut hits: Vec<usize> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| names_deleted(line, banned.name))
        .map(|(line_no, _)| line_no + 1)
        .collect();
    if let Some((prefix, leaf)) = banned.name.rsplit_once("::") {
        for offset in group_offsets(text, prefix, leaf) {
            let line_no = text[..offset].matches('\n').count() + 1;
            if !hits.contains(&line_no) {
                hits.push(line_no);
            }
        }
    }
    hits.sort_unstable();
    for line_no in hits {
        problems.push(format!(
            "{}:{}: `{}` is gone ({}): {}",
            path.display(),
            line_no,
            banned.name,
            banned.issue,
            banned.reason
        ));
    }
}

#[test]
fn deleted_names_stay_deleted() {
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
                for banned in TABLE {
                    scan_file(&file, &text, banned, &mut problems);
                }
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
