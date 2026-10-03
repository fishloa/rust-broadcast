//! Tripwire against hand-rolled generic protocol code coming back (W1 spec §5).
//!
//! Scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies for: HTTP/RTSP status-line literals,
//! a CRLFCRLF header terminator, `find("://")`, `strip_prefix("<scheme>://")`, SDP line building
//! (`"a=`, `"m=`, `"v=0`), civil-date helpers, a base64 alphabet literal, and
//! `thread::sleep` / `tokio::time::sleep(` in non-test code.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper or a string assembled from
//! pieces evades it. Code review is the real control.

use std::fs;
use std::path::Path;

/// (file suffix, needle, first line, last line, reason): an exemption is valid only for hits
/// inside the pinned line range of that file, so the allowlist cannot silently grow.
const ALLOW: &[(&str, &str, usize, usize, &str)] = &[
    (
        "src/rfc2326_lex.rs",
        "tokeniser",
        108,
        108,
        "owner decision (c), RFC 2326 §15.1 lexer",
    ),
    (
        "src/rfc2326_lex.rs",
        "tokeniser",
        218,
        218,
        "owner decision (c), RFC 2326 §15.1 lexer",
    ),
    (
        "src/rfc2326_lex.rs",
        "iteration",
        88,
        88,
        "owner decision (c), RFC 2326 §15.1 lexer: quote-aware separator scan",
    ),
    (
        "src/rfc2326_lex.rs",
        "iteration",
        193,
        193,
        "owner decision (c), RFC 2326 §15.1 lexer: digit validation",
    ),
    (
        "src/rfc2326_lex.rs",
        "iteration",
        205,
        205,
        "owner decision (c), RFC 2326 §15.1 lexer: hex-digit validation",
    ),
    (
        "src/framing.rs",
        "\\r\\n\\r\\n",
        27,
        27,
        "bounded framing check (finds the header terminator to enforce the 64 KiB cap and parse once); all header/body interpretation stays in rtsp_types",
    ),
];

const NEEDLES: &[&str] = &[
    "\"HTTP/1.",
    "\"RTSP/1.0 ",
    "\\r\\n\\r\\n",
    "find(\"://\")",
    "strip_prefix(\"rtsp://\")",
    "strip_prefix(\"rtmp://\")",
    "strip_prefix(\"http://\")",
    "\"a=",
    "\"m=",
    "\"v=0",
    "civil_from_days",
    "days_from_civil",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    "thread::sleep",
    "tokio::time::sleep(",
];

/// Methods that tokenise text around a separator.
const TOKENISERS: &[&str] = &[
    "split",
    "splitn",
    "rsplit",
    "rsplitn",
    "split_once",
    "rsplit_once",
    "find",
    "rfind",
    "split_terminator",
    "matches",
    "match_indices",
    "rmatches",
    "rmatch_indices",
    "strip_prefix",
    "strip_suffix",
];

/// Separator arguments that mean "hand-rolling a header tokeniser": the `;` `,` `=` `-` `/`
/// characters (char or one-char string literal) and the lexer's named separator constants.
const SEPARATOR_ARGS: &[&str] = &[
    "';'",
    "','",
    "'='",
    "'-'",
    "'/'",
    "\";\"",
    "\",\"",
    "\"=\"",
    "\"-\"",
    "\"/\"",
    "_SEP",
    "SLASH",
    "\"timeout=\"",
];

/// `.method(args)` calls on the line whose method is a tokeniser and whose arguments hold a
/// separator argument. Returns one description per hit. Lexical, like the rest of this guard.
fn tokenising_calls(line: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for m in TOKENISERS {
        let pat = format!(".{m}(");
        let mut from = 0;
        while let Some(i) = line[from..].find(&pat) {
            let args_start = from + i + pat.len();
            let mut depth = 1;
            let mut end = line.len();
            for (k, c) in line[args_start..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = args_start + k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let args = &line[args_start..end];
            if SEPARATOR_ARGS.iter().any(|a| args.contains(a)) {
                hits.push(format!("tokeniser .{m}({args})"));
            }
            from = args_start;
        }
    }
    hits
}

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

/// Source with every `#[cfg(test)] mod name { ... }` item removed (and an out-of-line
/// `#[cfg(test)] mod name;` declaration), found with a small lexer that understands
/// comments, strings (including multi-line and raw strings) and char literals, so an
/// unbalanced brace or quote inside a string cannot desynchronise it.
fn non_test_source(src: &str) -> String {
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

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let mut files = Vec::new();
    rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut hits = Vec::new();
    for (path, src) in &files {
        let body = non_test_source(src);
        for (n, line) in body.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue; // doc/comment text may quote the patterns
            }
            for hit in tokenising_calls(line) {
                if !ALLOW.iter().any(|(f, nd, lo, hi, _)| {
                    path.ends_with(f) && *nd == "tokeniser" && (*lo..=*hi).contains(&(n + 1))
                }) {
                    hits.push(format!("{path}:{}: {hit}", n + 1));
                }
            }
            // A hand-written char/byte scan loop is how a tokeniser hides from the call
            // patterns above; only the lexer (and pinned non-header uses) may have one.
            for pat in [".char_indices()", ".bytes()"] {
                if line.contains(pat)
                    && !ALLOW.iter().any(|(f, nd, lo, hi, _)| {
                        path.ends_with(f) && *nd == "iteration" && (*lo..=*hi).contains(&(n + 1))
                    })
                {
                    hits.push(format!("{path}:{}: char/byte iteration {pat}", n + 1));
                }
            }
            for needle in NEEDLES {
                if line.contains(needle)
                    && !ALLOW.iter().any(|(f, nd, lo, hi, _)| {
                        path.ends_with(f) && nd == needle && (*lo..=*hi).contains(&(n + 1))
                    })
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
    assert!(body.contains("HTTP/1."), "non-test line kept");
    assert_eq!(
        body.matches("HTTP/1.").count(),
        1,
        "test-module body removed"
    );
}

#[test]
fn an_out_of_line_test_mod_does_not_swallow_following_code() {
    let src = "#[cfg(test)]\nmod tests;\nfn real() { let x = \"HTTP/1.1 200\"; }\nfn other() { }\n";
    let body = non_test_source(src);
    assert!(
        body.contains("HTTP/1."),
        "code after `mod tests;` is kept: {body:?}"
    );
}

#[test]
fn braces_and_quotes_inside_strings_do_not_desynchronise_the_scanner() {
    let src = concat!(
        "#[cfg(test)]\nmod tests {\n",
        "    const A: &str = \"unbalanced { and \\\" quote\n continued } }\";\n",
        "    const B: &str = r#\"raw { \" } }\"#;\n",
        "    fn c() { let _ = '{'; let _: &'static str = \"x\"; }\n",
        "}\nfn real() { let x = \"HTTP/1.1 200\"; }\n"
    );
    let body = non_test_source(src);
    assert!(body.contains("fn real()"), "{body:?}");
    assert!(!body.contains("unbalanced"), "{body:?}");
}
