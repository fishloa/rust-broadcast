//! Tripwire guard (issue #2533, B4.6): multimux publishes segments with
//! `try_publish_segment`, never `publish_segment`, which parks its thread on a
//! `Condvar` under an `ArchiveOverrun::StallIngest` pin and so must never run
//! on an async worker. Scans multimux's `src/**/*.rs` outside `#[cfg(test)]`
//! items for `.publish_segment(`.
//!
//! **This is a lexical tripwire, not a proof.** A renamed wrapper evades it;
//! code review is the real control.

use workspace_guards::src_files;

const CRATE: &str = "multimux";
const BANNED: &str = ".publish_segment(";

fn offending_lines(files: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (rel, text) in files {
        for (no, line) in workspace_guards::scan::non_test_source_blanked(text)
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim_start().starts_with("//"))
        {
            if line.contains(BANNED) {
                out.push(format!("{rel}:{}: {}", no + 1, line.trim()));
            }
        }
    }
    out
}

#[test]
fn multimux_never_calls_the_blocking_publish_segment() {
    let files = src_files(CRATE);
    assert!(!files.is_empty(), "{CRATE}: no source files under src/");
    let hits = offending_lines(&files);
    assert!(
        hits.is_empty(),
        "use try_publish_segment (publish_segment can park the thread):\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_guard_bites() {
    let files = vec![(
        "src/x.rs".to_string(),
        "fn f(w: &W) {\n    w.publish_segment(e);\n    w.try_publish_segment(e);\n}\n".to_string(),
    )];
    assert_eq!(offending_lines(&files).len(), 1);
}
