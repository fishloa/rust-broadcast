//! Guard: a table crate reads XML only through `quick_xml` events (straight into
//! typed structs) and writes it only through `quick_xml::Writer`. Scans every
//! `src/**/*.rs` outside `#[cfg(test)]` code and fails on:
//!
//! - a struct/enum/type whose name contains the CamelCase word `Node`,
//!   `Element`, `Dom`, `Tree` or `Xml`, unless it is in the crate's `allowed_types` (each
//!   entry says why it is not an XML tree);
//! - a `fn` whose name contains `escape` defined in an XML module (a file that
//!   uses `quick_xml`);
//! - the entity literals `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` (a
//!   hand-rolled escape/unescape chain);
//! - `roxmltree`;
//! - a `fn` named with `esc`, `entity` or `xml` in an XML module, unless in `allowed_fns`;
//! - a match/if arm on `'&'`, `'<'` or `'>'` that pushes a string literal;
//! - XML-looking string building (`"<?xml`, `push_str("<`, `format!("<`) in a
//!   file that does not use `quick_xml`.
//!
//! **This is a lexical tripwire, not a proof.** It catches the telltale shapes
//! (a type or fn named like an XML tree/escaper, an entity literal, an
//! escape-table arm, hand-built markup); a renamed type (`Tag2`), a fn called
//! `esc2`, an untyped nested `Vec` tree or an entity table without the
//! literals can evade it. Code review is the real control.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// `(type name, reason)`: names containing a banned word that are not an
    /// XML tree/DOM.
    pub allowed_types: &'static [(&'static str, &'static str)],
    /// `(fn name, reason)`: functions in XML modules whose names contain
    /// `esc`, `entity` or `xml` but that are plumbing around quick-xml, not an
    /// escaper/unescaper.
    pub allowed_fns: &'static [(&'static str, &'static str)],
    /// Whether at least one `src/` file must use `quick_xml` (false for a
    /// crate that reaches XML only through another crate, so the scan is not
    /// vacuous by construction).
    pub needs_xml_module: bool,
}

const BANNED_WORDS: [&str; 5] = ["Node", "Element", "Dom", "Tree", "Xml"];
const ENTITY_LITERALS: [&str; 5] = ["&amp;", "&lt;", "&gt;", "&quot;", "&apos;"];

/// The production code of a file as `(line number, line)` pairs: comment lines
/// are dropped and every test-only item (`#[cfg(test)]`, `#[cfg(all(test,
/// ..))]`) is blanked with the shared lexer, so production code after a
/// mid-file test module is still scanned and every line number still matches
/// the file.
fn code_lines(text: &str) -> Vec<(usize, String)> {
    workspace_guards::scan::without_attributed_items(
        text,
        workspace_guards::scan::test_only_attribute,
    )
    .lines()
    .enumerate()
    .filter(|(_, line)| !line.trim_start().starts_with("//"))
    .map(|(no, line)| (no + 1, line.to_string()))
    .collect()
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

fn problems_in(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut hits = Vec::new();
    let mut xml_modules = 0;
    for (path, text) in files {
        let path = format!("{}: {path}", c.name);
        let uses_quick_xml = text.contains("quick_xml");
        if uses_quick_xml {
            xml_modules += 1;
        }
        for (n, line) in code_lines(text) {
            let line = line.as_str();
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
                if banned && !c.allowed_types.iter().any(|(n, _)| *n == name) {
                    hits.push(at(&format!("type `{name}` looks like an XML tree; allowlist it with a reason if it is not")));
                }
            }
            if uses_quick_xml && let Some(name) = declared_fn(line) {
                let lower = name.to_ascii_lowercase();
                if ["esc", "entity", "xml"].iter().any(|w| lower.contains(w))
                    && !c.allowed_fns.iter().any(|(n, _)| *n == name)
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
    if c.needs_xml_module && xml_modules == 0 {
        hits.push(format!(
            "{}: no file uses quick_xml: scan is vacuous",
            c.name
        ));
    }
    for (allowed, _) in c.allowed_types {
        let declared = files
            .iter()
            .flat_map(|(_, text)| code_lines(text))
            .filter(|(_, line)| declared_type(line).as_deref() == Some(*allowed))
            .count();
        if declared == 0 {
            hits.push(format!(
                "{}: allowlisted type `{allowed}` is not declared anywhere (stale allowlist entry)",
                c.name
            ));
        }
    }
    hits
}

#[test]
fn src_has_no_dom_or_hand_rolled_escaper() {
    let mut hits = Vec::new();
    for c in crate::no_dom_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no files under src", c.name);
        hits.extend(problems_in(c, &files));
    }
    assert!(hits.is_empty(), "XML guard violation(s): {hits:#?}");
}

#[test]
fn table_names_are_workspace_members() {
    let names: Vec<&str> = crate::no_dom_data::TABLE.iter().map(|c| c.name).collect();
    crate::common::assert_names_are_members("no_dom", &names);
}

/// Regression: `end_of_block` must find the closing brace of a MULTI-LINE
/// `#[cfg(test)] mod`, so production code AFTER it is still scanned (a per-line
/// `opened` made the scanner skip everything to EOF).
#[test]
fn code_lines_scans_past_multiline_cfg_test_mod() {
    let src = "fn a() {}\n\
        #[cfg(test)]\n\
        mod tests {\n\
        \x20   use super::*;\n\
        \x20   #[test]\n\
        \x20   fn t() {\n\
        \x20       assert!(true);\n\
        \x20   }\n\
        }\n\
        struct XmlNode;\n\
        fn z() { let _ = \"x\".replace(\"&amp;\", \"&\"); }\n";
    let code = code_lines(src);
    assert!(
        code.iter().any(|(_, l)| l.contains("struct XmlNode;")),
        "offender after a multi-line cfg(test) mod was skipped: {code:?}"
    );
    assert!(code.iter().any(|(_, l)| l.contains(".replace(\"&amp;\"")));
    assert!(
        !code.iter().any(|(_, l)| l.contains("assert!(true)")),
        "test-mod body must still be skipped"
    );
}

#[test]
fn problems_in_flags_a_dom_a_hand_escaper_and_a_stale_allow() {
    let c = Crate {
        name: "probe",
        allowed_types: &[("XmlGone", "stale")],
        allowed_fns: &[],
        needs_xml_module: true,
    };
    let files = vec![(
        "src/a.rs".to_string(),
        "use quick_xml::Reader;\nstruct XmlNode;\nfn z() { let _ = \"x\".replace(\"&amp;\", \"&\"); }\n"
            .to_string(),
    )];
    let hits = problems_in(&c, &files).join("\n");
    assert!(hits.contains("type `XmlNode`"), "{hits}");
    assert!(hits.contains("entity literal `&amp;`"), "{hits}");
    assert!(hits.contains("`XmlGone` is not declared"), "{hits}");
    let no_quick_xml = vec![("src/a.rs".to_string(), "fn a() {}\n".to_string())];
    assert!(
        problems_in(&c, &no_quick_xml)
            .join("\n")
            .contains("scan is vacuous")
    );
}

#[test]
fn a_test_module_in_the_middle_does_not_hide_the_code_after_it() {
    let src = "#[cfg(test)]\nmod t {\n    fn f() {}\n}\nfn g(s: &str) -> String {\n    s.replace(\"&amp;\", \"&\")\n}\n";
    let code = code_lines(src);
    assert!(
        code.iter()
            .any(|(no, line)| *no == 6 && line.contains("&amp;")),
        "offender after a mid-file test module was skipped: {code:?}"
    );
}
