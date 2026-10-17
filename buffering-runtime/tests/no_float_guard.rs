//! Lexical guard: no `f32`/`f64` (and no float literal) anywhere in `src`
//! outside `#[cfg(test)]` items. The crate is integer-only so its decisions
//! are bit-exact and mirror one-to-one in a C ABI. Also checks that every
//! `pub struct` with public fields is `#[non_exhaustive]` (the one documented
//! exception is the `#[repr(C)]` plain-data `EventRecord`).
//!
//! Lexical tripwire, not a proof: review is the real control.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn non_test_lines(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut lines = src.lines().enumerate();
    while let Some((no, line)) = lines.next() {
        let t = line.trim_start();
        if t.starts_with("#[cfg(test)]") {
            let (mut depth, mut opened) = (0i32, false);
            for (_, item) in lines.by_ref() {
                for ch in item.chars() {
                    if ch == '{' {
                        depth += 1;
                        opened = true;
                    } else if ch == '}' {
                        depth -= 1;
                    }
                }
                if opened && depth <= 0 {
                    break;
                }
            }
            continue;
        }
        if t.starts_with("//") {
            continue;
        }
        out.push((no + 1, line.to_string()));
    }
    out
}

fn has_float_token(line: &str) -> bool {
    let b: Vec<char> = line.chars().collect();
    for i in 0..b.len() {
        for ty in ["f32", "f64"] {
            let t: Vec<char> = ty.chars().collect();
            if b[i..].starts_with(&t) {
                let before_ok = i == 0 || !(b[i - 1].is_alphanumeric() || b[i - 1] == '_');
                let after = b.get(i + 3);
                let after_ok = after.is_none_or(|c| !(c.is_alphanumeric() || *c == '_'));
                if before_ok && after_ok {
                    return true;
                }
            }
        }
        // float literal: digit '.' digit
        if i + 2 < b.len() && b[i].is_ascii_digit() && b[i + 1] == '.' && b[i + 2].is_ascii_digit()
        {
            let prev_ident = i > 0 && (b[i - 1].is_alphanumeric() || b[i - 1] == '_');
            if !prev_ident {
                return true;
            }
        }
    }
    false
}

#[test]
fn src_has_no_floating_point() {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(!files.is_empty());
    let mut hits = Vec::new();
    for f in files {
        let text = fs::read_to_string(&f).unwrap();
        for (no, l) in non_test_lines(&text) {
            if has_float_token(&l) {
                hits.push(format!("{}:{no}: {}", f.display(), l.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "floating point in src:\n{}",
        hits.join("\n")
    );
}

#[test]
fn scanner_bites() {
    assert!(has_float_token("let x: f64 = 1;"));
    assert!(has_float_token("let y = 0.98;"));
    assert!(has_float_token("fn a(v: f32) {}"));
    assert!(!has_float_token("let leaf64 = 3;"));
}

const STRUCT_SKIP: &[&str] = &["EventRecord"];

#[test]
fn public_field_structs_are_non_exhaustive() {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut missing = Vec::new();
    for f in files {
        let text = fs::read_to_string(&f).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let t = l.trim_start();
            let Some(rest) = t.strip_prefix("pub struct ") else {
                continue;
            };
            if !t.trim_end().ends_with('{') {
                continue;
            }
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let has_pub_field = lines[i + 1..]
                .iter()
                .take_while(|x| x.trim() != "}")
                .any(|x| x.trim_start().starts_with("pub "));
            let attrs = &lines[i.saturating_sub(8)..i];
            let ne = attrs
                .iter()
                .any(|x| x.trim_start().starts_with("#[non_exhaustive]"));
            if has_pub_field && !ne && !STRUCT_SKIP.contains(&name.as_str()) {
                missing.push(name);
            }
        }
    }
    assert!(
        missing.is_empty(),
        "pub structs missing #[non_exhaustive]: {missing:?}"
    );
}
