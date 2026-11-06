//! Tripwire against hand-rolled generic protocol code coming back (W1 spec §5).
//!
//! Scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies for: HTTP/RTSP status-line literals,
//! a CRLFCRLF header terminator, `find("://")`, `strip_prefix("<scheme>://")`, SDP line building
//! (`"a=`, `"m=`, `"v=0`), civil-date helpers, a base64 alphabet literal, and
//! `thread::sleep` / `tokio::time::sleep(` in non-test code.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper or a string assembled from
//! pieces evades it. Code review is the real control.

use workspace_guards::scan::non_test_source;
use workspace_guards::src_files;

/// (file suffix, needle, first line, last line, reason): an exemption is valid only for hits
/// inside the pinned line range of that file, so the allowlist cannot silently grow.
/// `src/rfc2326_lex.rs` is exempted as a WHOLE FILE (`1..=usize::MAX`): it IS the
/// owner-approved RFC 2326 §15.1 lexer, and its exact line numbers shift on any
/// edit, so pinning them individually is a false tripwire. Every other file stays
/// fully guarded.
const ALLOW: &[(&str, &str, usize, usize, &str)] = &[
    (
        "src/rfc2326_lex.rs",
        "tokeniser",
        1,
        usize::MAX,
        "owner decision (c), RFC 2326 §15.1 lexer",
    ),
    (
        "src/rfc2326_lex.rs",
        "iteration",
        1,
        usize::MAX,
        "owner decision (c), RFC 2326 §15.1 lexer",
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

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let files = src_files("rtsp-runtime");
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
