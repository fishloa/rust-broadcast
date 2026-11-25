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

/// `true` when `name` occurs in `text` as a whole identifier rather than as
/// the prefix of a longer one (`CmafMux` must not match `CmafMuxStage`). A
/// qualified name (one containing `::`) is matched as written.
fn names_deleted(text: &str, name: &str) -> bool {
    if name.contains("::") {
        return text.contains(name);
    }
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
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

fn scan_file(path: &std::path::Path, text: &str, banned: &Banned, problems: &mut Vec<String>) {
    for (line_no, line) in text.lines().enumerate() {
        if names_deleted(line, banned.name) {
            problems.push(format!(
                "{}:{}: `{}` is gone ({}): {}",
                path.display(),
                line_no + 1,
                banned.name,
                banned.issue,
                banned.reason
            ));
        }
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
