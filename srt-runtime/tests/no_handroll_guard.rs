//! Tripwire against hand-rolled generic protocol code coming back (W1 spec §5).
//!
//! Scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies for: HTTP status-line literals, a CRLFCRLF
//! header terminator, `find("://")`, `strip_prefix("<scheme>://")`, SDP line building (`"a=`, `"m=`,
//! `"v=0`), civil-date helpers, a base64 alphabet literal, and `thread::sleep` / `sleep(` in
//! non-test async code.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper or a string assembled from pieces
//! evades it. Code review is the real control.
//!
//! Known limits: raw strings containing quotes and `/* */` comments are not
//! understood by the brace counter, and needles are lexical (a tokeniser built
//! on `split(',')` is not detected).

use std::fs;
use std::path::Path;

/// (file suffix, needle, reason).
const ALLOW: &[(&str, &str, &str)] = &[];

const NEEDLES: &[&str] = &[
    "\"HTTP/1.",
    "\\r\\n\\r\\n",
    "find(\"://\")",
    "strip_prefix(\"http://\")",
    "strip_prefix(\"https://\")",
    "strip_prefix(\"stun:\")",
    "\"a=",
    "\"m=",
    "\"v=0",
    "civil_from_days",
    "days_from_civil",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    "thread::sleep",
    "tokio::time::sleep(",
];

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

fn non_test_source(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("#[cfg(test)]")
            && let Some(m) = next_item_is_mod(&lines, i)
        {
            // An out-of-line `#[cfg(test)] mod x;` has no body here: skip only
            // its own line, never the next real block.
            i = if lines[m].trim_end().ends_with(';') {
                m + 1
            } else {
                block_end(&lines, m) + 1
            };
            continue;
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

fn next_item_is_mod(lines: &[&str], attr: usize) -> Option<usize> {
    let mut j = attr + 1;
    while j < lines.len() && lines[j].trim_start().starts_with("#[") {
        j += 1;
    }
    let t = lines.get(j)?.trim_start();
    let t = t.strip_prefix("pub ").unwrap_or(t);
    t.starts_with("mod ").then_some(j)
}

fn block_end(lines: &[&str], from: usize) -> usize {
    let (mut depth, mut seen_open) = (0i64, false);
    for (k, line) in lines.iter().enumerate().skip(from) {
        let (d, opened) = brace_delta(line);
        depth += d;
        seen_open |= opened;
        if seen_open && depth <= 0 {
            return k;
        }
    }
    lines.len() - 1
}

fn brace_delta(line: &str) -> (i64, bool) {
    let (mut delta, mut seen) = (0i64, false);
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
                let rest: String = chars.clone().take(3).collect();
                if let Some(end) = rest.find('\'') {
                    for _ in 0..=end {
                        chars.next();
                    }
                }
            }
            '{' => {
                delta += 1;
                seen = true;
            }
            '}' => delta -= 1,
            _ => {}
        }
    }
    (delta, seen)
}

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let mut files = Vec::new();
    rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut hits = Vec::new();
    for (path, src) in &files {
        for (n, line) in non_test_source(src).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for needle in NEEDLES {
                if line.contains(needle)
                    && !ALLOW
                        .iter()
                        .any(|(f, nd, _)| path.ends_with(f) && nd == needle)
                {
                    hits.push(format!("{path}:{}: {needle}", n + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "hand-rolled protocol code found (allowlist with a reason, or use the crate):\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_scanner_itself_bites() {
    let src = "fn real() { let x = \"HTTP/1.1 200\"; }\n#[cfg(test)]\nmod tests {\n    fn t() { let y = \"HTTP/1.1 200\"; }\n}\n";
    let body = non_test_source(src);
    assert_eq!(
        body.matches("HTTP/1.").count(),
        1,
        "test-module body removed, non-test line kept"
    );

    let out_of_line = "#[cfg(test)]\nmod tests;\nfn real() { let x = \"HTTP/1.1 200\"; }\n";
    assert_eq!(
        non_test_source(out_of_line).matches("HTTP/1.").count(),
        1,
        "an out-of-line test mod must not swallow the next real item"
    );
}
