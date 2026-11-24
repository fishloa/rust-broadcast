//! ISO BMFF box syntax lives in exactly one crate (streaming-first Phase 1).
//!
//! Every ISO/IEC 14496-12:2026 box header, four-CC literal and size-driven walk is
//! `isobmff`'s. A workspace member that hand-rolls one is drift. This guard
//! scans every crate's non-test `src/` code (except `isobmff` itself) and
//! fails on:
//!
//! - a four-character box-type byte literal from [`BOX_TYPES`], or the same
//!   four characters written as a `[u8; 4]` byte-char array;
//! - a local `const <name>` that re-declares one of the header sizes,
//!   sentinels or the FullBox extension under a name `isobmff::box_types`
//!   exports ([`HEADER_CONSTANTS`]).
//!
//! A crate that needs one of these uses `isobmff`'s constant. The one
//! exception is `container-probe`, which must depend only on
//! `broadcast-common` at runtime: it keeps its own tiny header walk and is
//! pinned to `isobmff` by its dev-only drift tests, so its lines are
//! allowlisted individually with that reason.
//!
//! **This is a lexical tripwire, not a proof.** A four-CC built at runtime, a
//! raw string, a renamed constant or a macro can evade it; code review is the
//! real control. Every allowlist entry is `(crate-relative path, exact trimmed
//! line, reason)` and a stale entry (no longer matching a banned line) fails.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// `(path suffix, exact trimmed line, reason)` — reviewed, line-pinned
    /// exceptions.
    pub allow: &'static [(&'static str, &'static str, &'static str)],
}

/// The four-CC byte literals no crate outside `isobmff` may spell itself.
pub const BOX_TYPES: &[[u8; 4]] = &[
    *b"ftyp", *b"styp", *b"moov", *b"moof", *b"mdat", *b"trak", *b"traf", *b"tfhd", *b"tfdt",
    *b"trun", *b"sidx", *b"mvhd", *b"tkhd", *b"mdia", *b"minf", *b"stbl", *b"stsd", *b"emsg",
    *b"prft", *b"free", *b"skip", *b"uuid", *b"senc", *b"saiz", *b"saio", *b"pssh", *b"sinf",
    *b"tenc",
];

/// Header size / sentinel constants `isobmff::box_types` exports; a local
/// `const` under one of these names is a re-declaration.
pub const HEADER_CONSTANTS: &[&str] = &[
    "BOX_HEADER_MIN_SIZE",
    "LARGESIZE_SIZE",
    "SIZE_TO_EOF",
    "SIZE_INDICATES_LARGESIZE",
    "FULLBOX_EXTRA_SIZE",
];

/// The byte-literal spelling of `fourcc`, e.g. `b"ftyp"`.
fn literal_needle(fourcc: &[u8; 4]) -> String {
    let mut out = String::from("b\"");
    for b in fourcc {
        out.push(char::from(*b));
    }
    out.push('"');
    out
}

/// The byte-char-array spelling of `fourcc` with whitespace and the char
/// quotes removed, e.g. `[b,b,b,b]` for both `[b'f', b't', b'y', b'p']` and
/// its spaced forms. Compared against a line with comments and string literals
/// already blanked, so only a real array can produce it.
fn array_needle(fourcc: &[u8; 4]) -> String {
    let mut out = String::from("["); // [b,b,b,b]
    for i in 0..fourcc.len() {
        if i > 0 {
            out.push(',');
        }
        out.push('b');
    }
    out.push(']');
    out
}

fn compact(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The line number (1-based) of byte `at`.
fn line_of(text: &str, at: usize) -> usize {
    text[..at].bytes().filter(|b| *b == b'\n').count() + 1
}

fn check(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut used = vec![false; c.allow.len()];

    let mut record = |rel: &str, no: usize, line: &str, why: &str, used: &mut Vec<bool>| {
        let trimmed = line.trim();
        if let Some(i) = c
            .allow
            .iter()
            .position(|(s, l, _)| rel.ends_with(s) && *l == trimmed)
        {
            used[i] = true;
        } else {
            violations.push(format!("{}: {rel}:{no}: {why}: {trimmed}", c.name));
        }
    };

    for (rel, text) in files {
        let text = workspace_guards::scan::non_test_source_blanked(text);
        let masked = workspace_guards::scan::blank_comments_and_strings(&text);

        for fourcc in BOX_TYPES {
            let needle = literal_needle(fourcc);
            for (at, _) in text.match_indices(&needle) {
                // The `b` of a real byte-string literal survives the mask; a
                // match inside a comment or an ordinary string starts on a
                // blank.
                if masked.as_bytes().get(at) != Some(&b' ') {
                    let no = line_of(&text, at);
                    let line = text.lines().nth(no - 1).unwrap_or("");
                    record(rel, no, line, "a box-type four-CC literal", &mut used);
                }
            }
        }

        for (idx, mline) in masked.lines().enumerate() {
            let squeezed = compact(mline);
            if BOX_TYPES
                .iter()
                .any(|f| squeezed.contains(&array_needle(f)))
            {
                let line = text.lines().nth(idx).unwrap_or("");
                record(
                    rel,
                    idx + 1,
                    line,
                    "a box-type four-CC byte-char array",
                    &mut used,
                );
            }
            for name in HEADER_CONSTANTS {
                if mline.contains(&format!("const {name}")) {
                    let line = text.lines().nth(idx).unwrap_or("");
                    record(
                        rel,
                        idx + 1,
                        line,
                        "a re-declared box-header constant",
                        &mut used,
                    );
                }
            }
        }
    }
    for (a, u) in c.allow.iter().zip(&used) {
        if !*u {
            violations.push(format!(
                "{}: stale ALLOW entry: ({}, {:?})",
                c.name, a.0, a.1
            ));
        }
    }
    violations
}

#[test]
fn no_hand_rolled_isobmff_box_syntax_outside_isobmff() {
    let mut violations = Vec::new();
    for c in crate::one_box_home_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        violations.extend(check(c, &files));
    }
    assert!(
        violations.is_empty(),
        "ISO BMFF box syntax outside `isobmff` (use its constants and iterator, or add a reasoned line-pinned ALLOW entry in one_box_home_data.rs):\n{}",
        violations.join("\n")
    );
}

#[test]
fn table_covers_every_workspace_member() {
    let names: Vec<&str> = crate::one_box_home_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    crate::common::assert_table_covers_members(
        "one_box_home",
        &names,
        crate::one_box_home_data::EXEMPT,
    );
}

#[test]
fn detector_bites() {
    let c = Crate {
        name: "synthetic",
        allow: &[("src/ok.rs", "const MOOV: [u8; 4] = *b\"moov\";", "test")],
    };
    let files = vec![
        (
            "src/a.rs".to_string(),
            "fn f() { let t = b\"moov\"; }\n".to_string(),
        ),
        (
            "src/b.rs".to_string(),
            "#[cfg(test)]\nmod tests {\n    const T: [u8; 4] = *b\"moov\";\n}\n".to_string(),
        ),
        (
            "src/c.rs".to_string(),
            "// b\"moov\" in a comment is fine\nconst S: &str = \"b\\\"moov\\\"\";\n".to_string(),
        ),
        (
            "src/d.rs".to_string(),
            "const BOX_HEADER_MIN_SIZE: usize = 8;\n".to_string(),
        ),
        (
            "src/e.rs".to_string(),
            "fn g() { let t = [b'f', b't', b'y', b'p']; }\n".to_string(),
        ),
        (
            "src/ok.rs".to_string(),
            "const MOOV: [u8; 4] = *b\"moov\";\n".to_string(),
        ),
    ];
    let violations = check(&c, &files);
    assert_eq!(violations.len(), 3, "{violations:?}");
    assert!(violations.iter().any(|v| v.contains("src/a.rs:1")));
    assert!(violations.iter().any(|v| v.contains("src/d.rs:1")));
    assert!(violations.iter().any(|v| v.contains("src/e.rs:1")));
}

#[test]
fn a_stale_allowlist_entry_fails() {
    let c = Crate {
        name: "synthetic",
        allow: &[("src/ok.rs", "const GONE: [u8; 4] = *b\"moov\";", "test")],
    };
    let files = vec![("src/ok.rs".to_string(), "fn f() {}\n".to_string())];
    let violations = check(&c, &files);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(
        violations[0].contains("stale ALLOW entry"),
        "{violations:?}"
    );
}
