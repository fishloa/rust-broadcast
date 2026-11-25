//! Names a finished refactor deleted may not reappear anywhere in a crate's
//! `src/`, `tests/` or `examples/`.
//!
//! The streaming-first Phase 3 slice 6 removed the whole-input MP4 demuxers and
//! replaced them with `transmux::mp4_demux::Mp4Demux`. A test, example or doc
//! comment that names a deleted type is either stale or a partial revert; both
//! are caught here. This is a lexical tripwire, not a proof: a name built at
//! runtime or spelled in a macro evades it, so code review remains the real
//! control.

use workspace_guards::{crate_dir, rs_files, workspace_members};

use crate::deleted_names_data::{Banned, EXEMPT, TABLE};

fn scan_file(path: &std::path::Path, text: &str, banned: &Banned, problems: &mut Vec<String>) {
    for (line_no, line) in text.lines().enumerate() {
        if line.contains(banned.name) {
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
