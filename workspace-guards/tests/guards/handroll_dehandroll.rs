//! Tripwire guard (spec §5, SP2–SP5), shape 1: generic protocol and format work in
//! each table crate goes through an established crate, not hand-rolled code. Scans every
//! `src/**/*.rs` outside `#[cfg(test)]` items and fails on:
//!
//! - an HTTP status/request line literal (`"HTTP/1.`) or a hand-framed
//!   `"\r\n\r\n"` header terminator;
//! - hand-split URLs: `find("://")`, `split_once("://")`,
//!   `strip_prefix("<scheme>://")`;
//! - hand-built SDP: a string literal starting `"v=0`, `"a=` or `"m=`;
//! - `civil_from_days` / `days_from_civil` (a hand-rolled calendar);
//! - a base64 alphabet literal;
//! - `thread::sleep` / `sleep(` in non-test code.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper, a split
//! literal or a macro can evade it; code review is the real control. Every
//! `ALLOW` entry states why the hit is not hand-rolled protocol work, and a
//! stale entry (one that no longer matches) fails the test.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// `(path suffix, needle, reason)` — reviewed exceptions.
    pub allow: &'static [(&'static str, &'static str, &'static str)],
}

/// `(needle, why it is banned)`. `strip_prefix("<scheme>://")` is checked
/// separately in `banned_in_line`.
const BANNED: &[(&str, &str)] = &[
    ("\"HTTP/1.", "hand-built HTTP status/request line"),
    ("\\r\\n\\r\\n", "hand-framed header terminator"),
    ("find(\"://\")", "hand-split URL"),
    ("split_once(\"://\")", "hand-split URL"),
    ("\"v=0", "hand-built SDP"),
    ("\"a=", "hand-built SDP attribute"),
    ("\"m=", "hand-built SDP media line"),
    ("civil_from_days", "hand-rolled calendar"),
    ("days_from_civil", "hand-rolled calendar"),
    (
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
        "base64 alphabet literal",
    ),
    ("thread::sleep", "blocking sleep in non-test code"),
    ("sleep(", "sleep in non-test code"),
];

/// The source with every `#[cfg(test)]` item (a `mod … { … }` block, or a single
/// line item) and every `//` comment line removed. Braces inside string
/// literals can fool the brace counter; that is within the tripwire's tolerance.
fn non_test_lines(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut lines = src.lines().enumerate().peekable();
    while let Some((no, line)) = lines.next() {
        let t = line.trim_start();
        if t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test") {
            let mut depth = 0i32;
            let mut opened = false;
            for (_, item) in lines.by_ref() {
                for c in item.chars() {
                    match c {
                        '{' => {
                            depth += 1;
                            opened = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if (opened && depth <= 0) || (!opened && item.trim_end().ends_with(';')) {
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

fn banned_in_line(line: &str) -> Vec<(&'static str, &'static str)> {
    let mut hits: Vec<_> = BANNED
        .iter()
        .filter(|(needle, _)| line.contains(needle))
        .copied()
        .collect();
    if line.contains("strip_prefix(\"") && line.contains("://\")") {
        hits.push(("strip_prefix(\"<scheme>://\")", "hand-split URL"));
    }
    hits
}

fn check(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut used = vec![false; c.allow.len()];
    for (rel, text) in files {
        for (no, line) in non_test_lines(text) {
            for (needle, why) in banned_in_line(&line) {
                if let Some(i) = c
                    .allow
                    .iter()
                    .position(|(suffix, n, _)| rel.ends_with(suffix) && *n == needle)
                {
                    used[i] = true;
                } else {
                    violations.push(format!("{}: {rel}:{no}: {why}: {}", c.name, line.trim()));
                }
            }
        }
    }
    for (a, u) in c.allow.iter().zip(&used) {
        if !*u {
            violations.push(format!("{}: stale ALLOW entry: ({}, {})", c.name, a.0, a.1));
        }
    }
    violations
}

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let mut violations = Vec::new();
    for c in crate::handroll_data::DEHANDROLL {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        violations.extend(check(c, &files));
    }
    assert!(
        violations.is_empty(),
        "hand-rolled protocol code (use the established crate, or add a reasoned ALLOW entry in handroll_data.rs):\n{}",
        violations.join("\n")
    );
}

/// The scanner itself: it must bite on the shapes it claims to catch and
/// ignore test-only code, so the guard cannot silently stop guarding.
#[test]
fn scanner_catches_banned_shapes_and_skips_test_items() {
    let src = "fn a() { let _ = \"HTTP/1.1 200\"; }\n\
               #[cfg(test)]\nmod tests {\n    fn t() { let _ = \"HTTP/1.1 404\"; }\n}\n\
               fn b() { s.strip_prefix(\"rtsp://\"); }\n\
               // \"v=0 in a comment\n";
    let hits: Vec<_> = non_test_lines(src)
        .into_iter()
        .flat_map(|(_, l)| banned_in_line(&l))
        .collect();
    assert_eq!(hits.len(), 2, "{hits:?}");
}

#[test]
fn stale_allow_entry_is_reported() {
    let c = Crate {
        name: "synthetic",
        allow: &[("src/gone.rs", "sleep(", "stale on purpose")],
    };
    let v = check(&c, &[("src/a.rs".into(), "fn a() { sleep(d); }\n".into())]);
    assert_eq!(v.len(), 2, "{v:?}");
    assert!(v[0].starts_with("synthetic: src/a.rs:1:"), "{v:?}");
}
