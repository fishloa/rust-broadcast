//! Workspace-wide lexical guards (unpublished).
//!
//! The guard tests live in `tests/guards.rs`; each guard family walks every
//! member crate named in its own per-crate data table and applies one shared
//! scanner. A new crate opts in by adding a line to that table, not by copying
//! a test file.

use std::fs;
use std::path::{Path, PathBuf};

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace-guards sits one level below the workspace root")
        .to_path_buf()
}

pub fn crate_dir(name: &str) -> PathBuf {
    workspace_root().join(name)
}

/// Every `.rs` under `dir`, sorted so reports are deterministic.
pub fn rs_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// `(path relative to the crate dir with `/` separators, contents)` for every
/// `.rs` under the crate's `src/`.
pub fn src_files(crate_name: &str) -> Vec<(String, String)> {
    let root = crate_dir(crate_name);
    rs_files(&root.join("src"))
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(&root)
                .expect("under crate dir")
                .to_string_lossy()
                .replace('\\', "/");
            let text = fs::read_to_string(&p).expect("read source");
            (rel, text)
        })
        .collect()
}

/// The `members` of the root `Cargo.toml`.
pub fn workspace_members() -> Vec<String> {
    let text = fs::read_to_string(workspace_root().join("Cargo.toml")).expect("root Cargo.toml");
    let start = text.find("members").expect("members key");
    let open = start + text[start..].find('[').expect("members [");
    let close = open + text[open..].find(']').expect("members ]");
    text[open + 1..close]
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
