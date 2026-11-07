//! Lexical tripwire: no NEW hand-written fixture loader.
//!
//! A test that needs a committed fixture calls `test_fixtures::read` /
//! `read_to_string` / `path` instead of building a `CARGO_MANIFEST_DIR` path
//! in its own `fn fixture()` / `read_fixture()` / `fixture_path()`. The files
//! below predate the helper and use shapes it does not yet cover
//! (workspace-relative or crate-local fixture paths, per-name subdir
//! logic); migrate one and delete it from the list. The list may only shrink.

use std::path::{Path, PathBuf};

const ALLOWED: &[&str] = &[];

const LOADER_NAMES: &[&str] = &[
    "fixture",
    "read_fixture",
    "fixture_path",
    "load_fixture",
    "fixture_bytes",
];

const SKIP_DIRS: &[&str] = &[
    "target",
    ".git",
    "node_modules",
    "private",
    "test-fixtures",
    "bindings",
    "fuzz",
    ".claude",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(&p, out);
            }
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

/// True if any loader-named fn in `s` has a body mentioning CARGO_MANIFEST_DIR.
fn offends(s: &str) -> bool {
    for name in LOADER_NAMES {
        let needle = format!("fn {name}(");
        let mut from = 0;
        while let Some(i) = s[from..].find(&needle) {
            let start = from + i;
            from = start + needle.len();
            let Some(open) = s[start..].find('{') else {
                continue;
            };
            let mut depth = 0usize;
            let mut end = s.len();
            for (k, c) in s[start + open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = start + open + k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if s[start..end].contains("CARGO_MANIFEST_DIR") {
                return true;
            }
        }
    }
    false
}

#[test]
fn no_new_handwritten_fixture_loaders() {
    let root = workspace_root();
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut bad = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(s) = std::fs::read_to_string(&f) else {
            continue;
        };
        if offends(&s) && !ALLOWED.contains(&rel.as_str()) {
            bad.push(rel);
        }
    }
    assert!(
        bad.is_empty(),
        "hand-written fixture loader(s) - use test_fixtures::read/read_to_string/path: {bad:?}"
    );
}

#[test]
fn allowlist_only_names_files_that_still_offend() {
    let root = workspace_root();
    for rel in ALLOWED {
        let s = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|_| panic!("allowlisted {rel} is gone - delete it from ALLOWED"));
        assert!(
            offends(&s),
            "{rel} no longer offends - delete it from ALLOWED"
        );
    }
}
