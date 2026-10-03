//! Tripwire guard (spec §5, SP2–SP5): generic protocol and format work in this
//! crate goes through an established crate, not hand-rolled code. Scans every
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

use std::fs;
use std::path::{Path, PathBuf};

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

/// `(path suffix, needle, reason)` — reviewed exceptions. Empty by design.
const ALLOW: &[(&str, &str, &str)] = &[];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

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

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src_dir, &mut files);
    assert!(!files.is_empty());
    let mut violations = Vec::new();
    let mut used = vec![false; ALLOW.len()];
    for file in files {
        let text = fs::read_to_string(&file).expect("read source");
        let rel = file
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for (no, line) in non_test_lines(&text) {
            for (needle, why) in banned_in_line(&line) {
                if let Some(i) = ALLOW
                    .iter()
                    .position(|(suffix, n, _)| rel.ends_with(suffix) && *n == needle)
                {
                    used[i] = true;
                } else {
                    violations.push(format!("{rel}:{no}: {why}: {}", line.trim()));
                }
            }
        }
    }
    let stale: Vec<_> = ALLOW
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(a, _)| a.0)
        .collect();
    assert!(
        violations.is_empty(),
        "hand-rolled protocol code (use the established crate, or add a reasoned ALLOW entry):\n{}",
        violations.join("\n")
    );
    assert!(stale.is_empty(), "stale ALLOW entries: {stale:?}");
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
