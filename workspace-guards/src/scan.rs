//! Shared scanner for the lexical source guards: `non_test_source` strips
//! `#[cfg(test)] mod` items with a small lexer that understands comments,
//! strings (including multi-line and raw strings) and char literals, so a brace
//! or quote inside a string cannot desynchronise it. A guard crate that keeps
//! its own extra checks takes this as a dev-dependency instead of copying it.

use std::path::Path;

/// Source with every `#[cfg(test)] mod name { ... }` item removed (and an out-of-line
/// `#[cfg(test)] mod name;` declaration), found with a small lexer that understands
/// comments, strings (including multi-line and raw strings) and char literals, so an
/// unbalanced brace or quote inside a string cannot desynchronise it.
pub fn non_test_source(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut line_start = true;
    while i < b.len() {
        if line_start
            && src[i..]
                .trim_start_matches([' ', '\t'])
                .starts_with("#[cfg(test)]")
            && let Some(end) = test_mod_end(b, i)
        {
            i = end;
            continue;
        }
        line_start = b[i] == b'\n' || (line_start && (b[i] == b' ' || b[i] == b'\t'));
        let ch = src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// If the `#[cfg(test)]` at `i` is followed (past other attributes) by a `mod`,
/// the index just past that module (its closing brace, or the `;`).
fn test_mod_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = skip_ws(b, i);
    // one or more attributes
    while b[j..].starts_with(b"#[") {
        let mut depth = 0i32;
        while j < b.len() {
            match b[j] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        j = skip_ws(b, j);
    }
    if b[j..].starts_with(b"pub ") {
        j = skip_ws(b, j + 4);
    }
    if !b[j..].starts_with(b"mod ") {
        return None;
    }
    while j < b.len() && b[j] != b'{' && b[j] != b';' {
        j += 1;
    }
    match b.get(j) {
        Some(b';') => Some(j + 1),
        Some(b'{') => Some(skip_block(b, j)),
        _ => None,
    }
}

/// Index just past the `}` matching the `{` at `start`.
fn skip_block(b: &[u8], start: usize) -> usize {
    let (mut i, mut depth) = (start, 0i32);
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut d = 1;
                i += 2;
                while i < b.len() && d > 0 {
                    if b[i..].starts_with(b"/*") {
                        d += 1;
                        i += 2;
                    } else if b[i..].starts_with(b"*/") {
                        d -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            b'r' | b'b' if is_raw_string_start(b, i) => {
                i = skip_raw_string(b, i);
                continue;
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
                continue;
            }
            b'\'' => {
                // char literal ('x' or '\n') vs lifetime ('a)
                if b.get(i + 1) == Some(&b'\\') {
                    i += 2;
                    while i < b.len() && b[i] != b'\'' {
                        i += 1;
                    }
                    i += 1;
                } else if b.get(i + 2) == Some(&b'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

fn is_raw_string_start(b: &[u8], i: usize) -> bool {
    let mut j = i;
    if b[j] == b'b' {
        j += 1;
    }
    if b.get(j) != Some(&b'r') {
        return false;
    }
    j += 1;
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    b.get(j) == Some(&b'"') && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'))
}

fn skip_raw_string(b: &[u8], i: usize) -> usize {
    let mut j = i;
    if b[j] == b'b' {
        j += 1;
    }
    j += 1; // r
    let mut hashes = 0;
    while b[j] == b'#' {
        hashes += 1;
        j += 1;
    }
    j += 1; // opening quote
    let close: Vec<u8> = std::iter::once(b'"')
        .chain(std::iter::repeat_n(b'#', hashes))
        .collect();
    while j < b.len() && !b[j..].starts_with(&close) {
        j += 1;
    }
    j + close.len()
}

/// Source files that exist only as `#[cfg(test)] mod name;` bodies of another
/// file in `files` (`(path, text)`), as the paths a module declaration resolves
/// to.
pub fn out_of_line_test_modules(files: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (path, src) in files {
        let p = Path::new(path);
        let dir = if p.file_name().is_some_and(|n| n == "mod.rs") {
            p.parent().map(Path::to_path_buf)
        } else {
            p.parent().zip(p.file_stem()).map(|(d, s)| d.join(s))
        };
        let Some(dir) = dir else { continue };
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]")
                && let Some(m) = next_item_is_mod(&lines, i)
                && let Some(name) = lines[m]
                    .trim()
                    .trim_start_matches("pub ")
                    .strip_prefix("mod ")
                    .and_then(|r| r.strip_suffix(';'))
            {
                out.push(dir.join(format!("{name}.rs")).display().to_string());
            }
        }
    }
    out
}

pub fn next_item_is_mod(lines: &[&str], attr: usize) -> Option<usize> {
    let mut j = attr + 1;
    while j < lines.len() && lines[j].trim_start().starts_with("#[") {
        j += 1;
    }
    let t = lines.get(j)?.trim_start();
    let t = t.strip_prefix("pub ").unwrap_or(t);
    t.starts_with("mod ").then_some(j)
}
