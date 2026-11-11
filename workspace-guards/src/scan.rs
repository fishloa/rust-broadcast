//! Shared scanner for the lexical source guards: `non_test_source` strips the
//! items `#[cfg(test)]` compiles only for tests — `#[cfg(test)]` itself and
//! the test-only `all(..)` forms such as `#[cfg(all(test, unix))]`, by
//! [`test_only_attribute`] — with a small lexer that understands comments,
//! strings (including multi-line and raw strings) and char literals, so a brace
//! or quote inside a string cannot desynchronise it. A guard crate that keeps
//! its own extra checks takes this as a dev-dependency instead of copying it.

use std::path::Path;

/// Source with every test-only `#[cfg(...)] mod name { ... }` item removed (and an
/// out-of-line `#[cfg(test)] mod name;` declaration) — "test-only" is
/// [`test_only_attribute`] — found with a small lexer that understands
/// comments, strings (including multi-line and raw strings) and char literals, so an
/// unbalanced brace or quote inside a string cannot desynchronise it.
pub fn non_test_source(src: &str) -> String {
    non_test_source_masked(src, false)
}

/// [`non_test_source`], but every removed `#[cfg(test)]` item keeps its line
/// breaks as empty lines, so a line number in what is left is still the line
/// number in `src` — what a guard that reports `file:line` needs.
pub fn non_test_source_blanked(src: &str) -> String {
    non_test_source_masked(src, true)
}

fn non_test_source_masked(src: &str, blank: bool) -> String {
    without_attributed_items_blanked(src, test_only_attribute, blank)
}

/// Whether an attribute is exactly test-only: `#[cfg(test)]`, or an `all(..)`
/// whose every term is `test` or a platform term that only narrows a test
/// build (`unix`, `windows`). A term set that includes anything else
/// (`any(..)`, `not(..)`, a feature) is NOT test-only — that item is compiled
/// outside `cargo test` and its code must keep being scanned.
pub fn test_only_attribute(attr: &str) -> bool {
    let mut has_test = false;
    let mut all_ok = true;
    for word in attribute_words(attr) {
        match word {
            "test" => has_test = true,
            "all" | "cfg" | "unix" | "windows" => {}
            _ => all_ok = false,
        }
    }
    has_test && all_ok
}

/// The bare words of an attribute, with punctuation, brackets and quotes
/// dropped: `#[cfg(all(test, unix))]` yields `cfg all test unix`.
fn attribute_words(expr: &str) -> Vec<&str> {
    expr.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .collect()
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
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

/// Source with every item carrying an attribute `matches` accepts removed —
/// a `mod … { … }` block, an out-of-line `mod …;`, or a `fn … { … }`. Line
/// breaks of a removed item are kept as empty lines, so a line number in what
/// is left is the line number in `src`. Other attributes in the same stack
/// (`#[cfg(test)]` above `#[path = "…"]` above `#[ignore]` above `fn f()`) are
/// skipped, and the item is found with the same string/comment-aware lexer as
/// [`non_test_source`].
pub fn without_attributed_items(src: &str, matches: impl Fn(&str) -> bool) -> String {
    without_attributed_items_blanked(src, matches, true)
}

/// [`without_attributed_items`], with the removed items either blanked (their
/// line breaks kept) or cut entirely.
fn without_attributed_items_blanked(
    src: &str,
    matches: impl Fn(&str) -> bool,
    blank: bool,
) -> String {
    let b = src.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut line_start = true;
    while i < b.len() {
        if line_start && let Some(end) = attributed_item_end(src, b, i, &matches) {
            if blank {
                out.extend(src[i..end].chars().filter(|c| *c == '\n'));
            }
            i = end;
            line_start = true;
            continue;
        }
        line_start = b[i] == b'\n' || (line_start && (b[i] == b' ' || b[i] == b'\t'));
        let ch = src[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// If the line starting at `i` opens an attribute stack whose attributes
/// `matches` accepts and ends in an item, the index just past that item.
fn attributed_item_end(
    src: &str,
    b: &[u8],
    i: usize,
    matches: &impl Fn(&str) -> bool,
) -> Option<usize> {
    let mut j = i;
    let mut found = false;
    loop {
        let rest = src.get(j..)?;
        let indent = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        j += indent;
        if !b[j..].starts_with(b"#[") {
            break;
        }
        let start = j;
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
        found |= matches(&src[start..j]);
        j = skip_ws(b, j);
    }
    found.then(|| item_end(b, j))?
}

/// Index just past the item whose head starts at `j`: a `;`-terminated
/// declaration or a `{`-delimited body, scanning the head with the same lexer
/// so a `{` or `;` inside a string, a char literal, a comment or an angle
/// bracket cannot end it early.
fn item_end(b: &[u8], start: usize) -> Option<usize> {
    let mut j = start;
    while j < b.len() {
        match b[j] {
            b'/' if b.get(j + 1) == Some(&b'/') => {
                while j < b.len() && b[j] != b'\n' {
                    j += 1;
                }
            }
            b'/' if b.get(j + 1) == Some(&b'*') => j = skip_comment(b, j),
            b'r' | b'b' if is_raw_string_start(b, j) => j = skip_raw_string(b, j),
            b'"' => {
                j += 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                j += 1;
            }
            b'\'' => j = skip_char_literal(b, j),
            b';' => return Some(j + 1),
            b'{' => return Some(skip_block(b, j)),
            _ => j += 1,
        }
    }
    None
}

/// Index just past the char literal starting at `i` (a lifetime `'a` is not
/// one and is left to the caller).
fn skip_char_literal(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    if b.get(j) == Some(&b'\\') {
        j += 2;
    } else if b.get(j).is_some_and(u8::is_ascii) {
        j += 1;
    }
    if b.get(j) == Some(&b'\'') {
        j + 1
    } else {
        i + 1
    }
}

/// Index just past the block comment starting at `i`.
fn skip_comment(b: &[u8], i: usize) -> usize {
    let (mut j, mut depth) = (i + 2, 1);
    while j < b.len() && depth > 0 {
        if b[j..].starts_with(b"/*") {
            depth += 1;
            j += 2;
        } else if b[j..].starts_with(b"*/") {
            depth -= 1;
            j += 2;
        } else {
            j += 1;
        }
    }
    j
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
