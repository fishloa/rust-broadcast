//! Lexical tripwire (streaming-first Phase 6): nothing reads a whole input.
//!
//! Every media capture is streamed through a container stage or scanner in
//! bounded chunks, and every small document goes through
//! `broadcast_common::bounded` with a named [`broadcast_common::bounded::Limit`].
//! A new `fs::read(path)` of a capture compiles, passes every test on a small
//! fixture and exhausts a host's memory on a real file; this guard fails CI on
//! it. It also bans the whole-input IR (`transmux::Media`/`transmux::Track`)
//! and the deleted `broadcast_common::mux` traits, and keeps the dev-only
//! `test_harness` out of production `src/`.
//!
//! The scope is every workspace member's non-test `src/` and its `examples/`,
//! plus the out-of-workspace `demo/` and `tools/differential-sweep/` roots.
//! `#[cfg(test)]` code, `tests/` and `benches/` are out of scope. The one
//! allowlisted site is the body of `broadcast-common/src/bounded.rs`.
//!
//! **This is a lexical tripwire, not a proof.** A helper built at runtime or
//! spelled through a macro evades it; code review is the real control.

use workspace_guards::scan::{
    blank_comments_and_strings, test_starting_attribute, without_attributed_items,
};
use workspace_guards::{crate_dir, rs_files, src_files};

pub struct Crate {
    pub name: &'static str,
    /// `(file suffix, reason)`: every hit in a matching file is allowed.
    pub allow: &'static [(&'static str, &'static str)],
}

/// Whole-input read calls, matched as plain substrings. `read_to_end(` and
/// `read_to_string(` also match their method and trait forms
/// (`.read_to_end(`, `io::read_to_string(`, `AsyncReadExt::read_to_end(`), and
/// `fs::read(`/`fs::read_to_string(` also match the `tokio::fs` forms.
const READ_NEEDLES: &[&str] = &[
    "fs::read(",
    "fs::read_to_string(",
    "read_to_end(",
    "read_to_string(",
];

/// Whole paths, matched with a trailing identifier boundary so
/// `transmux::TrackSpec` does not count as `transmux::Track`.
const PATH_NEEDLES: &[&str] = &[
    "broadcast_common::mux",
    "transmux::Media",
    "transmux::Track",
    "transmux::ir::Media",
    "transmux::ir::Track",
];

/// The dev-only test bridge crate. It is never a production dependency.
const TEST_HARNESS: &str = "test_harness";

const OUTSIDE_WORKSPACE_ROOTS: &[&str] = &["demo", "tools/differential-sweep"];

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Whether `text` contains `needle` as a whole path: the byte after it is not
/// an identifier character.
fn contains_path(text: &str, needle: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(offset) = text[from..].find(needle) {
        let end = from + offset + needle.len();
        if bytes.get(end).is_none_or(|c| !is_ident(*c)) {
            return true;
        }
        from = end;
    }
    false
}

/// Whether `text` contains `word` as a whole identifier.
fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(offset) = text[from..].find(word) {
        let begin = from + offset;
        let end = begin + word.len();
        let starts = begin == 0 || !is_ident(bytes[begin - 1]);
        let ends = bytes.get(end).is_none_or(|c| !is_ident(*c));
        if starts && ends {
            return true;
        }
        from = end;
    }
    false
}

/// Line numbers of `use` statements that import `Media` or `Track` from
/// `transmux` as a group (`use transmux::{CmafMuxStage, Media};`). The direct
/// form `use transmux::Media;` is caught by [`PATH_NEEDLES`].
fn transmux_ir_imports(blanked: &str) -> Vec<usize> {
    let bytes = blanked.as_bytes();
    let mut hits = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let is_use = bytes[i..].starts_with(b"use")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 3).is_none_or(|c| !is_ident(*c));
        if is_use {
            let end = bytes[i..]
                .iter()
                .position(|&c| c == b';')
                .map(|offset| i + offset)
                .unwrap_or(bytes.len());
            let statement = &blanked[i..end];
            if contains_word(statement, "transmux")
                && (contains_word(statement, "Media") || contains_word(statement, "Track"))
            {
                hits.push(blanked[..i].matches('\n').count() + 1);
            }
            i = end + 1;
            continue;
        }
        i += 1;
    }
    hits
}

fn scan_text(text: &str, include_test_harness: bool) -> Vec<(usize, String)> {
    let non_test = without_attributed_items(text, test_starting_attribute);
    let body = blank_comments_and_strings(&non_test);
    let mut hits = Vec::new();
    for (index, line) in body.lines().enumerate() {
        let n = index + 1;
        let mut reported = false;
        for needle in READ_NEEDLES {
            if line.contains(needle) {
                hits.push((n, format!("`{needle}`")));
                reported = true;
                break;
            }
        }
        if !reported {
            for needle in PATH_NEEDLES {
                if contains_path(line, needle) {
                    hits.push((n, format!("`{needle}`")));
                    reported = true;
                    break;
                }
            }
        }
        if !reported && include_test_harness && contains_word(line, TEST_HARNESS) {
            hits.push((n, format!("`{TEST_HARNESS}`")));
        }
    }
    for n in transmux_ir_imports(&body) {
        if !hits.iter().any(|(line, _)| *line == n) {
            hits.push((n, "`Media`/`Track` imported from `transmux`".to_string()));
        }
    }
    hits.sort();
    hits.dedup();
    hits
}

fn allowed(c: &Crate, path: &str) -> bool {
    c.allow.iter().any(|(suffix, _)| path.ends_with(suffix))
}

fn scan_file(c: &Crate, path: &str, text: &str, include_test_harness: bool, out: &mut Vec<String>) {
    if allowed(c, path) {
        return;
    }
    for (n, needle) in scan_text(text, include_test_harness) {
        out.push(format!("{}: {path}:{n}: {needle}", c.name));
    }
}

fn example_files(name: &str) -> Vec<(String, String)> {
    let root = crate_dir(name);
    let dir = root.join("examples");
    if !dir.is_dir() {
        return Vec::new();
    }
    rs_files(&dir)
        .into_iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let rel = p
                .strip_prefix(&root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some((rel, text))
        })
        .collect()
}

fn scan_crate(c: &Crate, out: &mut Vec<String>) {
    for (path, text) in src_files(c.name) {
        scan_file(c, &path, &text, true, out);
    }
    for (path, text) in example_files(c.name) {
        scan_file(c, &path, &text, false, out);
    }
}

fn scan_plain_root(root: &str, out: &mut Vec<String>) {
    let dir = workspace_guards::workspace_root().join(root);
    if !dir.is_dir() {
        return;
    }
    for area in ["src", "examples"] {
        let area = dir.join(area);
        if !area.is_dir() {
            continue;
        }
        for file in rs_files(&area) {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let rel = file
                .strip_prefix(workspace_guards::workspace_root())
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| file.display().to_string());
            for (n, needle) in scan_text(&text, true) {
                out.push(format!("{root}: {rel}:{n}: {needle}"));
            }
        }
    }
}

#[test]
fn no_unbounded_input_in_non_test_source() {
    let mut problems = Vec::new();
    for c in crate::bounded_input_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        scan_crate(c, &mut problems);
    }
    for root in OUTSIDE_WORKSPACE_ROOTS {
        scan_plain_root(root, &mut problems);
    }
    assert!(
        problems.is_empty(),
        "a whole-input read, the whole-input IR, a deleted mux trait, or the dev-only \
         `test_harness` in production source (stream it through a stage, read a document \
         through `broadcast_common::bounded`, or allow-list in bounded_input_data.rs):\n{}",
        problems.join("\n")
    );
}

#[test]
fn table_covers_every_workspace_member() {
    let names: Vec<&str> = crate::bounded_input_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    crate::common::assert_table_covers_members(
        "bounded_input",
        &names,
        crate::bounded_input_data::EXEMPT,
    );
}

/// G3: the allowlist names exactly one file, so it cannot grow silently.
#[test]
fn the_allowlist_is_exactly_the_bounded_module() {
    let entries: Vec<(&str, &str)> = crate::bounded_input_data::TABLE
        .iter()
        .flat_map(|c| c.allow.iter().map(move |(suffix, _)| (c.name, *suffix)))
        .collect();
    assert_eq!(
        entries,
        vec![("broadcast-common", "src/bounded.rs")],
        "the one allowed whole-input read is the body of broadcast_common::bounded"
    );
}

/// G3: `EXEMPT` is exactly the publish = false test-support crates.
#[test]
fn the_exempt_list_is_exactly_the_test_support_crates() {
    assert_eq!(
        crate::bounded_input_data::EXEMPT,
        &[
            "test-fixtures",
            "test-harness",
            "test-alloc",
            "workspace-guards"
        ],
    );
}

#[test]
fn the_scanner_bites() {
    let planted = "fn f() {\n    let _ = std::fs::read(\"x\");\n}\n";
    assert_eq!(scan_text(planted, true).len(), 1);
    assert_eq!(
        scan_text("fn f() {\n    use transmux::Media;\n}\n", true).len(),
        1
    );
    assert_eq!(
        scan_text("fn f() {\n    let _ = file.read_to_end(&mut v);\n}\n", true).len(),
        1
    );
    assert_eq!(
        scan_text("fn f() {\n    tokio::fs::read_to_string(p);\n}\n", true).len(),
        1
    );
    assert_eq!(
        scan_text("use transmux::{\n    CmafMuxStage,\n    Media,\n};\n", true).len(),
        1,
        "a use group that imports Media from transmux must bite"
    );
    assert_eq!(
        scan_text(
            "fn f() {\n    let _ = broadcast_common::mux::Unpackage;\n}\n",
            true
        )
        .len(),
        1
    );
    assert_eq!(
        scan_text("fn f() {\n    test_harness::collect(x);\n}\n", true).len(),
        1
    );
}

#[test]
fn the_scanner_ignores_test_code_comments_and_strings() {
    assert!(
        scan_text(
            "fn f() {}\n#[cfg(test)]\nmod t {\n    fn g() { std::fs::read(\"x\"); }\n}\n",
            true
        )
        .is_empty()
    );
    assert!(scan_text("// std::fs::read(x)\nfn f() {}\n", true).is_empty());
    assert!(scan_text("fn f() {\n    let _ = \"std::fs::read(x)\";\n}\n", true).is_empty());
    assert!(scan_text("fn f() {\n    let _ = \"use transmux::Media;\";\n}\n", true).is_empty());
    assert!(scan_text("fn f() {\n    let _ = \"test_harness\";\n}\n", true).is_empty());
}

#[test]
fn the_scanner_does_not_match_a_replacement_name() {
    assert!(scan_text("fn f() -> transmux::TrackSpec {}\n", true).is_empty());
    assert!(scan_text("fn f() -> transmux::TrackId {}\n", true).is_empty());
    assert!(scan_text("use transmux::{TrackSpec, TrackId};\n", true).is_empty());
    assert!(scan_text("fn f() {\n    test_harness_copy();\n}\n", true).is_empty());
}

#[test]
fn an_allowlisted_file_is_skipped() {
    let c = Crate {
        name: "probe",
        allow: &[("src/bounded.rs", "reason")],
    };
    let mut out = Vec::new();
    scan_file(
        &c,
        "src/bounded.rs",
        "fn f() { let _ = fs::read(\"x\"); }\n",
        true,
        &mut out,
    );
    assert!(out.is_empty(), "{out:?}");
    scan_file(
        &c,
        "src/other.rs",
        "fn f() { let _ = fs::read(\"x\"); }\n",
        true,
        &mut out,
    );
    assert_eq!(out.len(), 1, "{out:?}");
}

#[test]
fn the_scanner_keeps_line_numbers() {
    let src = "#[cfg(test)]\nmod t {\n    fn g() {}\n}\nfn f() { let _ = std::fs::read(\"x\"); }\n";
    let hits = scan_text(src, true);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].0, 5, "the line number is the one in the file");
}

#[test]
fn every_needle_of_every_kind_bites() {
    for needle in READ_NEEDLES {
        let src = format!("fn f() {{ let _ = {needle}\"x\"); }}\n");
        assert_eq!(scan_text(&src, true).len(), 1, "{needle}");
    }
    for needle in PATH_NEEDLES {
        let src = format!("fn f() {{ let _ = {needle}::X; }}\n");
        assert_eq!(scan_text(&src, true).len(), 1, "{needle}");
    }
}

/// The scanner understands the shared lexer's limits: a raw string is blanked.
#[test]
fn the_scanner_survives_raw_strings() {
    let src =
        "fn f() {}\n#[cfg(test)]\nmod t {\n    const S: &str = r#\"std::fs::read(x) {}\"#;\n}\n";
    assert!(scan_text(src, true).is_empty(), "raw string in test module");
}
