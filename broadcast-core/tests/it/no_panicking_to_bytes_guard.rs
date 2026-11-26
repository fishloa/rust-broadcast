//! Guard (umbrella #1143 / T15): `Serialize` has no panicking `to_bytes`
//! default. The only allocate-and-serialize entry point is the fallible
//! `try_to_bytes`. A lexical tripwire, not a proof: it fails if the panicking
//! default (or its `expect` message) reappears in `traits.rs`, or if a
//! `#[allow(deprecated)]` shows up in any crate's non-test `src/` (the escape
//! hatch a deprecate-instead-of-remove regression would need).

use std::fs;
use std::path::{Path, PathBuf};

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if !matches!(name, "target" | "private" | "bindings" | ".git" | "tests") {
                rs_files(&p, out);
            }
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

/// Source with everything from the first `#[cfg(test)]` module on dropped.
fn non_test_part(src: &str) -> String {
    let mut out = String::new();
    let mut lines = src.lines().peekable();
    while let Some(l) = lines.next() {
        if l.trim() == "#[cfg(test)]"
            && lines
                .peek()
                .is_some_and(|n| n.trim_start().starts_with("mod ") || n.contains(" mod "))
        {
            break;
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

#[test]
fn serialize_trait_has_no_panicking_to_bytes() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/traits.rs");
    let src = non_test_part(&fs::read_to_string(p).unwrap());
    assert!(
        !src.contains("fn to_bytes"),
        "Serialize::to_bytes (panicking default) must not return; use try_to_bytes"
    );
    assert!(
        !src.contains(".expect(\"serialize_into"),
        "traits.rs must not panic on a serialize_into error"
    );
    assert!(src.contains("fn try_to_bytes"));
}

#[test]
fn no_allow_deprecated_in_production_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for e in fs::read_dir(root).unwrap().flatten() {
        let src = e.path().join("src");
        if src.is_dir() {
            rs_files(&src, &mut files);
        }
    }
    assert!(!files.is_empty(), "scan found no sources");
    let mut hits = Vec::new();
    for f in files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "tests.rs"
            || name.ends_with("_tests.rs")
            || name == "no_panicking_to_bytes_guard.rs"
        {
            continue;
        }
        let Ok(s) = fs::read_to_string(&f) else {
            continue;
        };
        let prod = non_test_part(&s);
        if prod.contains("allow(deprecated)") {
            hits.push(f.display().to_string());
        }
    }
    assert!(
        hits.is_empty(),
        "allow(deprecated) in production source: {hits:?}"
    );
}
