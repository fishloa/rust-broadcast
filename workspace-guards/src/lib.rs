//! Workspace-wide lexical guards (unpublished).
//!
//! The guard tests live in `tests/guards/main.rs`; each guard family walks every
//! member crate named in its own per-crate data table and applies one shared
//! scanner. A new crate opts in by adding a line to that table, not by copying
//! a test file.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

pub mod modules;
pub mod scan;

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
/// `.rs` under the crate's `src/` that is not compiled only for tests.
///
/// A file reached only through a test-only out-of-line module
/// (`#[cfg(test)] mod tests;`) is test code, and the guards exempt test code:
/// it keeps its line numbers in the file it came from, so it is left out here
/// rather than scanned as production source.
pub fn src_files(crate_name: &str) -> Vec<(String, String)> {
    let root = crate_dir(crate_name);
    let files: Vec<(PathBuf, String)> = rs_files(&root.join("src"))
        .into_iter()
        .map(|p| {
            let text = fs::read_to_string(&p).expect("read source");
            (p, text)
        })
        .collect();
    let test_only = test_only_module_files(&files);
    files
        .into_iter()
        .filter_map(|(p, text)| {
            let canonical = fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
            if test_only.contains(&canonical) {
                return None;
            }
            let rel = p
                .strip_prefix(&root)
                .expect("under crate dir")
                .to_string_lossy()
                .replace('\\', "/");
            Some((rel, text))
        })
        .collect()
}

/// The files under `src/` that are reachable from a `lib.rs`/`main.rs` only
/// through a test-only [`modules::Declaration`], with their paths canonical so
/// a `#[path]` naming `..` still matches the walked file.
fn test_only_module_files(files: &[(PathBuf, String)]) -> BTreeSet<PathBuf> {
    let texts: HashMap<PathBuf, &str> = files
        .iter()
        .filter_map(|(p, t)| fs::canonicalize(p).ok().map(|c| (c, t.as_str())))
        .collect();
    let mut prod: BTreeSet<PathBuf> = BTreeSet::new();
    let mut test: BTreeSet<PathBuf> = BTreeSet::new();
    for (path, _) in files {
        let is_crate_root = path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|d| d == "src")
            && path
                .file_name()
                .is_some_and(|n| n == "lib.rs" || n == "main.rs");
        if is_crate_root {
            visit_module(path, false, &texts, &mut prod, &mut test);
        }
    }
    test.difference(&prod).cloned().collect()
}

fn visit_module(
    file: &Path,
    test_only: bool,
    texts: &HashMap<PathBuf, &str>,
    prod: &mut BTreeSet<PathBuf>,
    test: &mut BTreeSet<PathBuf>,
) {
    let Ok(path) = fs::canonicalize(file) else {
        return;
    };
    if test_only {
        if prod.contains(&path) || !test.insert(path.clone()) {
            return;
        }
    } else if !prod.insert(path.clone()) {
        return;
    }
    let Some(text) = texts.get(&path) else {
        return;
    };
    for decl in modules::declarations(text) {
        let candidates = match &decl.path {
            Some(rel) => modules::resolve(&modules::path_base(&path), rel),
            None => modules::resolve(&modules::module_base(&path), &decl.name),
        };
        for candidate in candidates {
            if candidate.is_file() {
                visit_module(&candidate, test_only || decl.test_only, texts, prod, test);
                break;
            }
        }
    }
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
