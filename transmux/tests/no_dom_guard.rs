//! Guard: this crate reads XML only through `quick_xml` events (straight into
//! typed structs) and writes it only through `quick_xml::Writer`. Scans every
//! `src/**/*.rs` outside `#[cfg(test)]` code and fails on:
//!
//! - a struct/enum/type whose name contains the CamelCase word `Node`,
//!   `Element`, `Dom`, `Tree` or `Xml`, unless it is in `ALLOWED_TYPES` (each
//!   entry says why it is not an XML tree);
//! - a `fn` whose name contains `escape` defined in an XML module (a file that
//!   uses `quick_xml`);
//! - the entity literals `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` (a
//!   hand-rolled escape/unescape chain);
//! - `roxmltree`;
//! - a `fn` named with `esc`, `entity` or `xml` in an XML module, unless allowlisted;
//! - a match/if arm on `'&'`, `'<'` or `'>'` that pushes a string literal;
//! - XML-looking string building (`"<?xml`, `push_str("<`, `format!("<`) in a
//!   file that does not use `quick_xml`.
//!
//! **This is a lexical tripwire, not a proof.** It catches the telltale shapes
//! (a type or fn named like an XML tree/escaper, an entity literal, an
//! escape-table arm, hand-built markup); a renamed type (`Tag2`), a fn called
//! `esc2`, an untyped nested `Vec` tree or an entity table without the
//! literals can evade it. Code review is the real control.

use std::fs;
use std::path::Path;

/// Types whose names contain a banned word but are NOT an XML tree/DOM.
const ALLOWED_TYPES: &[(&str, &str)] = &[
    (
        "XmlReader",
        "private alias of quick_xml::Reader, not a tree",
    ),
    (
        "XmlWriter",
        "private alias of quick_xml::Writer<Vec<u8>>, not a tree",
    ),
    (
        "XmlSubtitleSampleEntry",
        "ISO BMFF stpp sample entry (XML subtitle track) box, unrelated to XML parsing",
    ),
];

/// Functions in XML modules whose names contain `esc`, `entity` or `xml` but
/// that are plumbing around quick-xml, not an escaper/unescaper.
const ALLOWED_FNS: &[(&str, &str)] = &[
    (
        "xml_error",
        "maps a quick-xml error to the parser error enum",
    ),
    (
        "xml_message",
        "builds a positioned parser error from a validation message",
    ),
    (
        "write_service_description",
        "writes <ServiceDescription>/<UTCTiming> as quick-xml events",
    ),
    (
        "is_xml_char",
        "XML 1.0 section 2.2 Char predicate used to validate decoded text, not an escaper",
    ),
];

const BANNED_WORDS: [&str; 5] = ["Node", "Element", "Dom", "Tree", "Xml"];
const ENTITY_LITERALS: [&str; 5] = ["&amp;", "&lt;", "&gt;", "&quot;", "&apos;"];

fn rs_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.display().to_string(),
                fs::read_to_string(&path).expect("read source"),
            ));
        }
    }
}

/// Net `{` minus `}` on a line, ignoring string/char literals and `//` comments.
fn brace_delta(line: &str) -> (i64, bool) {
    let mut delta = 0i64;
    let mut seen_open = false;
    let mut chars = line.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '/' if chars.peek() == Some(&'/') => break,
            '\'' => {
                // A char literal ('{', '\n', ...) — skip it; lifetimes have no closing quote.
                let rest: String = chars.clone().take(3).collect();
                if let Some(end) = rest.find('\'') {
                    for _ in 0..=end {
                        chars.next();
                    }
                }
            }
            '{' => {
                delta += 1;
                seen_open = true;
            }
            '}' => delta -= 1,
            _ => {}
        }
    }
    (delta, seen_open)
}

/// Whether the item after the attribute line at `attr` is a `mod`.
fn next_item_is_mod(lines: &[&str], attr: usize) -> Option<usize> {
    let mut j = attr + 1;
    while j < lines.len() && lines[j].trim_start().starts_with("#[") {
        j += 1;
    }
    let t = lines.get(j)?.trim_start();
    let t = t.strip_prefix("pub").map_or(t, |r| {
        let r = r.trim_start();
        if r.starts_with('(') {
            r.find(')').map_or(r, |i| r[i + 1..].trim_start())
        } else {
            r
        }
    });
    t.starts_with("mod ").then_some(j)
}

/// Index of the last line of the brace-matched block that starts on `from`
/// (`mod x;` with no body ends on its own line).
fn end_of_block(lines: &[&str], from: usize) -> usize {
    let mut depth = 0i64;
    for (k, line) in lines.iter().enumerate().skip(from) {
        let (d, opened) = brace_delta(line);
        depth += d;
        if opened && depth <= 0 {
            return k;
        }
        if !opened && depth == 0 && line.trim_end().ends_with(';') {
            return k;
        }
    }
    lines.len().saturating_sub(1)
}

/// The production code of a file with its `(line number, line)` pairs:
/// comment lines are dropped and ONLY the brace-matched body of a
/// `#[cfg(test)]`-annotated `mod` is skipped; everything else (including other
/// `#[cfg(test)]` items such as a `use` or a helper fn) is still scanned.
fn code_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if t.starts_with("#[cfg(test)]")
            && let Some(m) = next_item_is_mod(&lines, i)
        {
            i = end_of_block(&lines, m) + 1;
            continue;
        }
        if !t.starts_with("//") {
            out.push((i + 1, lines[i]));
        }
        i += 1;
    }
    out
}

/// The CamelCase words of an identifier (`XmlReader` -> `Xml`, `Reader`).
fn words(name: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() || words.is_empty() {
            words.push(c.to_string());
        } else if let Some(last) = words.last_mut() {
            last.push(c);
        }
    }
    words
}

/// The type name declared on `line`, if it declares a struct/enum/union/type.
fn declared_type(line: &str) -> Option<String> {
    let mut t = line.trim_start();
    if let Some(rest) = t.strip_prefix("pub") {
        t = rest.trim_start();
        if t.starts_with('(') {
            t = &t[t.find(')')? + 1..];
        }
        t = t.trim_start();
    }
    for kw in ["struct ", "enum ", "union ", "type "] {
        if let Some(rest) = t.strip_prefix(kw) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn declared_fn(line: &str) -> Option<String> {
    let idx = line.find("fn ")?;
    let before = &line[..idx];
    if !before.trim().is_empty()
        && !before.trim_end().ends_with("pub")
        && !before.contains("pub(")
        && !before.contains("unsafe")
        && !before.contains("const")
    {
        return None;
    }
    let name: String = line[idx + 3..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// A string literal that opens an XML tag (`"<name`, `"</`, `"<?`, `"<!`) in a
/// push/format call, i.e. markup being assembled by hand.
fn builds_xml_markup(line: &str) -> bool {
    for call in [
        "push_str(\"<",
        "format!(\"<",
        "push_str(&format!(\"<",
        "write!(out, \"<",
    ] {
        let mut rest = line;
        while let Some(i) = rest.find(call) {
            let after = &rest[i + call.len()..];
            if after
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '?' | '!'))
            {
                return true;
            }
            rest = after;
        }
    }
    line.contains("\"<?xml")
}

/// The line `n` (1-based) and the three after it, joined.
fn lines_after(text: &str, n: usize) -> String {
    text.lines()
        .skip(n - 1)
        .take(4)
        .collect::<Vec<_>>()
        .join("\n")
}

/// A match/if arm on the character `'&'`, `'<'` or `'>'` whose body pushes a
/// string literal: the shape of a hand-rolled escape table.
fn pushes_literal_for_markup_char(window: &str) -> bool {
    let first = window.lines().next().unwrap_or("");
    let on_markup_char = [
        "'&' =>", "'<' =>", "'>' =>", "== '&'", "== '<'", "== '>'", "'&' |", "'<' |", "'>' |",
    ]
    .iter()
    .any(|p| first.contains(p));
    on_markup_char && window.contains("push_str(\"")
}

#[test]
fn src_has_no_dom_or_hand_rolled_escaper() {
    let mut files = Vec::new();
    rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(!files.is_empty());
    let mut hits = Vec::new();
    let mut xml_modules = 0;
    for (path, text) in &files {
        let uses_quick_xml = text.contains("quick_xml");
        if uses_quick_xml {
            xml_modules += 1;
        }
        for (n, line) in code_lines(text) {
            let at = |msg: &str| format!("{path}:{n}: {msg}");
            if line.contains("roxmltree") {
                hits.push(at("`roxmltree`"));
            }
            for lit in ENTITY_LITERALS {
                if line.contains(lit) {
                    hits.push(at(&format!(
                        "entity literal `{lit}` (hand-rolled escape/unescape)"
                    )));
                }
            }
            if let Some(name) = declared_type(line) {
                let banned = words(&name)
                    .iter()
                    .any(|w| BANNED_WORDS.contains(&w.as_str()));
                if banned && !ALLOWED_TYPES.iter().any(|(n, _)| *n == name) {
                    hits.push(at(&format!("type `{name}` looks like an XML tree; allowlist it with a reason if it is not")));
                }
            }
            if uses_quick_xml && let Some(name) = declared_fn(line) {
                let lower = name.to_ascii_lowercase();
                if ["esc", "entity", "xml"].iter().any(|w| lower.contains(w))
                    && !ALLOWED_FNS.iter().any(|(n, _)| *n == name)
                {
                    hits.push(at(&format!(
                        "fn `{name}` in an XML module: escaping/entity/XML plumbing is quick-xml's (allowlist it with a reason if it is not)"
                    )));
                }
            }
            if pushes_literal_for_markup_char(&lines_after(text, n)) {
                hits.push(at("match/if arm on '&', '<' or '>' pushing a string literal (hand-rolled escaping)"));
            }
            if !uses_quick_xml && builds_xml_markup(line) {
                hits.push(at(
                    "XML-looking string building in a file that does not use quick_xml",
                ));
            }
        }
    }
    assert!(xml_modules > 0, "no file uses quick_xml: scan is vacuous");
    // Every allowlisted type is still declared somewhere (no stale entries).
    for (allowed, _) in ALLOWED_TYPES {
        let declared = files
            .iter()
            .flat_map(|(_, text)| code_lines(text))
            .filter(|(_, line)| declared_type(line).as_deref() == Some(*allowed))
            .count();
        if declared == 0 {
            hits.push(format!(
                "allowlisted type `{allowed}` is not declared anywhere (stale allowlist entry)"
            ));
        }
    }
    assert!(hits.is_empty(), "XML guard violation(s): {hits:#?}");
}
