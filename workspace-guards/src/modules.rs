//! Out-of-line `mod name;` declarations of a Rust source file, the directory
//! each resolves in, and whether the declaration is test-only
//! (`#[cfg(test)]` / `#[cfg(all(test, ..))]`).
//!
//! Shared by [`crate::src_files`], which leaves test-only modules out of the
//! lexical guards (they are test code), and by the guard that checks every
//! file under `tests/<bin>/` is reachable from its `main.rs`.

use std::path::{Path, PathBuf};

use crate::scan::test_only_attribute;

/// One out-of-line module declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// The declaration is compiled only for tests.
    pub test_only: bool,
    /// Its `#[path = "..."]`, relative to the declaring file's directory.
    pub path: Option<String>,
    /// The declared module name.
    pub name: String,
}

fn strip_visibility(line: &str) -> &str {
    line.strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub(super) "))
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line)
}

/// Every out-of-line `mod name;` in `text`, comments skipped and the
/// attributes directly above a declaration applied to it.
pub fn declarations(text: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    let mut attributes: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(inner) = line.strip_prefix("#[").and_then(|r| r.strip_suffix(']')) {
            attributes.push(inner.to_string());
            continue;
        }
        if let Some(name) = strip_visibility(line)
            .strip_prefix("mod ")
            .and_then(|r| r.strip_suffix(';'))
        {
            let name = name.trim();
            if !name.is_empty()
                && !name.as_bytes()[0].is_ascii_digit()
                && name.bytes().all(|b| b == b'_' || b.is_ascii_alphanumeric())
            {
                let test_only = attributes.iter().any(|attr| test_only_attribute(attr));
                let path = attributes.iter().find_map(|attr| {
                    let rest = attr.strip_prefix("path")?.trim_start();
                    let rest = rest.strip_prefix('=')?.trim();
                    let quoted = rest.strip_prefix('"')?;
                    Some(quoted.split_once('"')?.0.to_string())
                });
                out.push(Declaration {
                    test_only,
                    path,
                    name: name.to_string(),
                });
            }
        }
        attributes.clear();
    }
    out
}

/// Where `mod name;` looks: a `lib.rs`/`main.rs`/`mod.rs` module's directory,
/// or the directory named after a non-`mod.rs` file.
pub fn module_base(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new(""));
    let is_root = file
        .file_name()
        .is_some_and(|n| n == "lib.rs" || n == "main.rs" || n == "mod.rs");
    if is_root {
        parent.to_path_buf()
    } else {
        parent.join(file.file_stem().unwrap_or_default())
    }
}

/// Where a `#[path = ".."] mod ..;` looks: the source file's own directory.
pub fn path_base(file: &Path) -> PathBuf {
    file.parent().unwrap_or(Path::new("")).to_path_buf()
}

/// The candidate files a declaration names, in lookup order.
pub fn resolve(base: &Path, rel: &str) -> Vec<PathBuf> {
    if rel.ends_with(".rs") {
        return vec![base.join(rel)];
    }
    vec![
        base.join(format!("{rel}.rs")),
        base.join(rel).join("mod.rs"),
    ]
}
