//! Tripwire against hand-rolled generic protocol code coming back (W1 spec §5),
//! shape 3 (needle list with a (file suffix, needle, reason) allowlist).
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

use workspace_guards::scan::{non_test_source, out_of_line_test_modules};
use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// Needles scanned in addition to [`NEEDLES`].
    pub extra: &'static [&'static str],
    /// (file suffix, needle, reason).
    pub allow: &'static [(&'static str, &'static str, &'static str)],
}

/// Shared by every shape-3 crate.
pub const NEEDLES: &[&str] = &[
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

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let mut hits = Vec::new();
    for c in crate::handroll_data::NEEDLE_CRATES {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        let test_only = out_of_line_test_modules(&files);
        for (path, src) in &files {
            if test_only.contains(path) {
                continue;
            }
            for (n, line) in non_test_source(src).lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for needle in NEEDLES.iter().chain(c.extra) {
                    if line.contains(needle)
                        && !c
                            .allow
                            .iter()
                            .any(|(f, nd, _)| path.ends_with(f) && nd == needle)
                    {
                        hits.push(format!("{}: {path}:{}: {needle}", c.name, n + 1));
                    }
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "hand-rolled protocol code found (allowlist with a reason in handroll_data.rs, or use the crate):\n{}",
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

#[test]
fn every_needle_of_every_crate_bites() {
    for c in crate::handroll_data::NEEDLE_CRATES {
        for needle in NEEDLES.iter().chain(c.extra) {
            let src = format!("fn f() {{ let _ = {needle}; }}\n");
            assert!(
                non_test_source(&src).contains(needle),
                "{}: {needle}",
                c.name
            );
            let in_tests =
                format!("#[cfg(test)]\nmod tests {{\n    fn f() {{ let _ = {needle}; }}\n}}\n");
            assert!(
                !non_test_source(&in_tests).contains(needle),
                "{}: {needle} must be ignored inside a test mod",
                c.name
            );
        }
    }
}

#[test]
fn the_scanner_survives_strings_raw_strings_comments_and_non_ascii() {
    // A string in a PRODUCTION line containing an unbalanced brace must not
    // desync the checker: production code after the test module stays visible.
    let brace_in_str = "fn a() { let s = \"}\"; }\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\nfn prod_after() { let x = \"HTTP/1.1 200\"; }\n";
    assert_eq!(
        non_test_source(brace_in_str).matches("HTTP/1.").count(),
        1,
        "a production string with an unbalanced brace must not swallow prod code after the test mod"
    );

    // A RAW string containing braces inside the test module — the old
    // per-line `in_str` scanner desynced on these (the WHEP_TEST_OFFER shape).
    let raw_in_test = "fn a() {}\n#[cfg(test)]\nmod tests {\n    const O: &str = r#\"v=0\r\n{}{}\"#;\n    fn t() { let y = \"HTTP/1.1 200\"; }\n}\n";
    assert_eq!(
        non_test_source(raw_in_test).matches("HTTP/1.").count(),
        0,
        "a raw string with braces in the test module must still remove the whole module"
    );

    // A MULTI-LINE `\"...\"` string continuation inside the test module with a
    // stray `{` — also must not desync (a per-line scanner would).
    let multiline_str_in_test = "fn a() {}\n#[cfg(test)]\nmod tests {\n    const O: &str = \"v=0\r\n{\r\n\";\n    fn t() { let y = \"HTTP/1.1 200\"; }\n}\n";
    assert_eq!(
        non_test_source(multiline_str_in_test)
            .matches("HTTP/1.")
            .count(),
        0,
        "a multi-line string with a brace in the test module must still remove the module"
    );

    // A block comment containing braces must not desync brace depth.
    let block_comment = "fn a() { /* } { */ }\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\nfn prod() { let x = \"HTTP/1.1 200\"; }\n";
    assert_eq!(
        non_test_source(block_comment).matches("HTTP/1.").count(),
        1,
        "a comment brace must not swallow production code after the test mod"
    );

    // Non-ASCII text must survive byte-for-byte: the old `b[i] as char` walk
    // mangled every multi-byte character (U+00E9 -> two chars), which would
    // corrupt any needle or source line carrying non-ASCII (this workspace's
    // doc comments use em dashes and `§`).
    let non_ascii = "fn a() {}\n/// § \u{2014} café\n#[cfg(test)]\nmod tests {\n    fn t() { let y = \"HTTP/1.1 200\"; }\n}\nfn prod() { let x = \"HTTP/1.1 200\"; }\n";
    let stripped = non_test_source(non_ascii);
    assert!(
        stripped.contains("/// § \u{2014} café"),
        "non-ASCII text must be preserved intact, got: {stripped:?}"
    );
    assert_eq!(
        stripped.matches("HTTP/1.").count(),
        1,
        "non-ASCII text must not desync the scanner (only the test-mod body removed)"
    );
}
