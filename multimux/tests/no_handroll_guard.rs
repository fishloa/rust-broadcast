//! Tripwire against hand-rolled generic protocol code coming back (W2a spec §5).
//!
//! Scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies. Cross-references
//! `tests/harness_guard.rs` (W2b-1 Task 10), which guards THIS crate's test
//! harness (reserve-then-rebind loops, sleeps) — this file guards `src/`
//! only.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper or a string assembled from pieces
//! evades it. Code review is the real control.
//!
//! Known limits: raw strings containing quotes and `/* */` comments are not
//! understood by the brace counter, and needles are lexical (a tokeniser built
//! on `split(',')` is not detected).

use std::fs;
use std::path::Path;

/// A line-pinned allow: `(file suffix, needle, line marker, reason)`. The
/// needle is only allowed on a source line that ALSO contains `line_marker`, so
/// a new occurrence of the same needle elsewhere in the same file still fails
/// (the previous file+needle allowlist let any new `find("://")` in `redact.rs`
/// through).
const ALLOW: &[(&str, &str, &str, &str)] = &[
    // --- Redaction of a URL the `url` parser REJECTS (spec §9 documented
    // exception): the masking-only fallbacks scan the raw text for the `://`
    // boundary and the `@`. They do NOT reconstruct a secret from the text
    // (behaviourally guarded in-file by
    // `masking_fallbacks_do_not_reconstruct_a_secret`). The two `raw.find("://")`
    // sites are `redact_unparseable_userinfo` / `redact_unparseable_destination`.
    (
        "redact.rs",
        "find(\"://\")",
        "let Some(scheme_end) = raw.find(\"://\") else {",
        "redact.rs masking-only `://` boundary scan for an unparseable URL (spec §9)",
    ),
    (
        "redact.rs",
        "find(\"://\")",
        "if let Some(scheme_end) = url.find(\"://\") {",
        "redact.rs `scrub_destination_secrets`: the `://` boundary of the URL being scrubbed from an error message",
    ),
    (
        "redact.rs",
        "rfind('@')",
        "let Some(at) = authority.rfind('@') else {",
        "redact.rs masking-only fallback: locate the credential's `@` boundary (spec §9)",
    ),
    (
        "redact.rs",
        "find('@')",
        "let Some(at) = authority.rfind('@') else {",
        "`find('@')` is a substring of this `rfind('@')` line (same masking fallback site)",
    ),
    (
        "redact.rs",
        "rsplit_once('@')",
        "if let Some((userinfo, _host)) = authority.rsplit_once('@') {",
        "redact.rs `scrub_destination_secrets`: split authority userinfo from host for token scrubbing",
    ),
    (
        "redact.rs",
        "split_once('@')",
        "if let Some((userinfo, _host)) = authority.rsplit_once('@') {",
        "`split_once('@')` is a substring of this `rsplit_once('@')` line (same scrub site)",
    ),
    // --- SRT query split stays manual (W2b-1 Task 2): a Haivision
    // `streamid=#!::r=...` value contains a `#` that `url`'s `query_pairs()`
    // would cut as a fragment delimiter. Only the AUTHORITY parse moved to the
    // `url` crate; the scheme prefix is stripped here before `split_once('?')`.
    (
        "push/srt.rs",
        "strip_prefix(\"srt://\")",
        "let stripped = url.strip_prefix(\"srt://\").unwrap_or(url);",
        "push/srt.rs: the query split stays manual (Haivision streamid `#`)",
    ),
    // --- The RTSP push renders its own ANNOUNCE SDP (`build_sdp`); it moves
    // onto rtsp-runtime's adapter in W2b-2, not here. ---
    (
        "push/rtsp.rs",
        "\"v=0",
        "\"v=0\\r\\n\\",
        "push/rtsp.rs `build_sdp` renders the ANNOUNCE SDP (W2b-2 moves the RTSP push)",
    ),
];

/// (file suffix, needle, line marker, reason) — like [`ALLOW`] but for the
/// config.rs / whep.rs SDP test-fixture literals whose raw-string braces sit
/// inside `#[test]` fns. Pinning to the exact line still lets any other
/// `"v=0` in those files trip the guard.
const PINNED: &[(&str, &str, &str, &str)] = &[
    (
        "config.rs",
        "\"v=0",
        "let sdp = \"v=0\\r\\no=- 0 0 IN IP4",
        "config.rs `parses_json_config_with_rtp_input` test fixture",
    ),
    (
        "config.rs",
        "\"v=0",
        "let long_sdp = \"v=0\\r\\n\".repeat(50)",
        "config.rs `route_debug_shows_sdp_length_not_full_body` test fixture",
    ),
    (
        "output/whep.rs",
        "\"v=0",
        "pub const WHEP_TEST_OFFER: &str = \"v=0",
        "output/whep.rs `WHEP_TEST_OFFER` (W2b-1 Task 7 test fixture)",
    ),
];

const NEEDLES: &[&str] = &[
    "\"HTTP/1.",
    "\\r\\n\\r\\n",
    "find(\"://\")",
    "strip_prefix(\"srt://\")",
    "strip_prefix(\"rtsp://\")",
    "strip_prefix(\"rtmp://\")",
    "\"a=",
    "\"m=",
    "\"v=0",
    "civil_from_days",
    "days_from_civil",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    "thread::sleep",
    "Arc<TokioMutex<MediaTransport>>",
    "Arc<tokio::sync::Mutex<MediaTransport>>",
    // Userinfo/host splitting (I11): the whole family of hand splits is
    // caught, not just `rsplit_once('@')`, so a respelling cannot slip past.
    "rsplit_once('@')",
    "split_once('@')",
    "rfind('@')",
    "find('@')",
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

/// Remove every `#[cfg(test)] mod … { … }` block from `src`, leaving all other
/// text verbatim (needles live inside string literals, so string contents must
/// NOT be blanked — only the test-module span is cut).
///
/// The scanner is a full-len char walker tracking string / raw-string (any
/// `#` count) / char-literal / line-comment / (nested) block-comment state
/// ACROSS lines, so a multi-line string, a raw string containing braces, or a
/// `/* */` comment can no longer desync the brace depth (the previous
/// per-line `in_str` scanner did). An out-of-line `#[cfg(test)] mod x;`
/// removes only its own line.
fn non_test_source(src: &str) -> String {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = String::with_capacity(n);
    let mut i = 0usize;
    while i < n {
        // At a position where a `#[cfg(test)]` attribute could begin (start of
        // a line / after whitespace), try to match the test-mod attribute.
        if let Some((attr_start, mod_kw, body_open)) = match_cfg_test_mod(b, i) {
            // Emit everything before the attribute as-is.
            out.push_str(&src[i..attr_start]);
            if body_open.is_none() {
                // Out-of-line `mod x;`: skip just through its `;`.
                let semi = find_semicolon(b, mod_kw).unwrap_or(mod_kw);
                i = semi + 1;
                continue;
            }
            // Block `mod … { … }`: cut through the matching close brace.
            let open = body_open.unwrap();
            let end = match_close_brace(b, open).unwrap_or(n - 1);
            // Keep the attribute+mod line's leading text out; replace the whole
            // block with a newline to preserve line numbering reasonably.
            i = end + 1;
            out.push('\n');
            continue;
        }
        out.push(b[i] as char);
        // Advance a whole UTF-8 char to keep byte indices sane for ASCII source
        // (source here is ASCII Rust); a non-ASCII byte is emitted as its raw
        // byte to keep `i` in lockstep with the byte scan.
        i += 1;
    }
    out
}

/// If a `#[cfg(test)]` attribute starts at or after byte `from` (with only
/// whitespace between `from` and the `#`), find the `mod` keyword that follows
/// (skipping any additional `#[…]` attributes) and the `{` opening its body
/// (or `None` for an out-of-line `mod x;`). Returns
/// `(attr_start, mod_kw_pos, body_open)`.
#[allow(clippy::type_complexity)]
fn match_cfg_test_mod(b: &[u8], from: usize) -> Option<(usize, usize, Option<usize>)> {
    let n = b.len();
    // Only whitespace may precede the attribute on its line.
    let mut j = from;
    while j < n && (b[j] == b' ' || b[j] == b'\t') {
        j += 1;
    }
    if j + 12 > n || &b[j..j + 12] != b"#[cfg(test)]" {
        return None;
    }
    let attr_start = j;
    let mut k = j + 12;
    // Skip whitespace and any further attributes up to the `mod` keyword.
    loop {
        while k < n && (b[k] == b' ' || b[k] == b'\t' || b[k] == b'\r' || b[k] == b'\n') {
            k += 1;
        }
        if k < n && b[k] == b'#' && k + 1 < n && b[k + 1] == b'[' {
            let close = matching_bracket(b, k + 1)?;
            k = close + 1;
            continue;
        }
        break;
    }
    let rest = &b[k..];
    if rest.starts_with(b"pub ") {
        k += 4;
        while k < n && (b[k] == b' ' || b[k] == b'\t') {
            k += 1;
        }
    }
    if !b[k..].starts_with(b"mod ") {
        return None;
    }
    let mod_kw = k;
    let mut m = k + 4;
    while m < n && (b[m] as char).is_alphanumeric() || (m < n && b[m] == b'_') {
        m += 1;
    }
    // Find `{` or `;`, handling generics-free mod only (no generic mods).
    while m < n && b[m] != b'{' && b[m] != b';' {
        m += 1;
    }
    if m >= n {
        return None;
    }
    if b[m] == b';' {
        return Some((attr_start, mod_kw, None));
    }
    Some((attr_start, mod_kw, Some(m)))
}

/// The byte index of the `[`'s matching `]`, respecting string/char/comment
/// state (a `]` inside a string is ignored).
fn matching_bracket(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    let mut state = ScanState::Normal;
    while i < b.len() {
        let (ns, consumed) = advance_state(b, i, &mut state);
        if state == ScanState::Normal {
            match b[i] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        state = ns;
        i += 1 + consumed;
    }
    None
}

/// The byte index of the `{`'s matching `}` in CODE (braces inside strings /
/// comments / raw strings are ignored), or `None`.
fn match_close_brace(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    let mut state = ScanState::Normal;
    let mut seen_open = false;
    while i < b.len() {
        let (ns, consumed) = advance_state(b, i, &mut state);
        if state == ScanState::Normal {
            match b[i] {
                b'{' => {
                    depth += 1;
                    seen_open = true;
                }
                b'}' => {
                    depth -= 1;
                    if seen_open && depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        state = ns;
        i += 1 + consumed;
    }
    None
}

/// Scan state for the len-aware brace/bracket walkers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Normal,
    /// Block comment, with the nesting depth.
    BlockComment(u32),
}

/// Given the state BEFORE `b[i]`, return the state AFTER consuming byte `i`
/// and how many EXTRA bytes were consumed (a string/comment span). Only the
/// states needed for brace-safe scanning are modelled: line comments end at
/// `\n`; strings (`"…"`) and raw strings (`r#"…"#`) and char literals skip
/// their contents; block comments nest.
fn advance_state(b: &[u8], i: usize, state: &mut ScanState) -> (ScanState, usize) {
    // Continue a block comment across lines: this function is called per byte
    // with `i` advancing, so a block comment is handled one byte at a time
    // here (no extra consumption) to keep the loop simple.
    if let ScanState::BlockComment(depth) = *state {
        if i + 1 < b.len() && b[i] == b'*' && b[i + 1] == b'/' {
            return (
                if depth <= 1 {
                    ScanState::Normal
                } else {
                    ScanState::BlockComment(depth - 1)
                },
                1,
            );
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            return (ScanState::BlockComment(depth + 1), 1);
        }
        return (ScanState::BlockComment(depth), 0);
    }
    // Line comment: swallow to end of line.
    if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
        let mut k = i;
        while k < b.len() && b[k] != b'\n' {
            k += 1;
        }
        return (ScanState::Normal, k.saturating_sub(i).saturating_sub(1));
    }
    // Block comment start.
    if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
        return (ScanState::BlockComment(1), 1);
    }
    // Raw string: r"..", r#".."#, br#".."# etc.
    let raw = raw_string_span(b, i);
    if let Some(end) = raw {
        return (ScanState::Normal, end.saturating_sub(i));
    }
    // Normal string.
    if b[i] == b'"' {
        let mut k = i + 1;
        while k < b.len() {
            if b[k] == b'\\' {
                k += 2;
                continue;
            }
            if b[k] == b'"' {
                break;
            }
            k += 1;
        }
        return (ScanState::Normal, k.saturating_sub(i));
    }
    // Char literal: 'x' or '\n' etc. Distinguish from a lifetime `'a` by
    // requiring a closing quote within a few bytes.
    if b[i] == b'\'' {
        let mut k = i + 1;
        if k < b.len() && b[k] == b'\\' {
            k += 1;
        }
        if k < b.len() {
            k += 1;
        }
        if k < b.len() && b[k] == b'\'' {
            return (ScanState::Normal, k - i);
        }
    }
    (ScanState::Normal, 0)
}

/// If a raw string starts at `i` (`r"…"`, `r#"…"#`, `br#"…"#`, …), return the
/// index of its closing `"` + hashes + (for `b` prefix) accounted last byte.
fn raw_string_span(b: &[u8], i: usize) -> Option<usize> {
    let mut k = i;
    if k < b.len() && b[k] == b'b' {
        k += 1;
    }
    if k >= b.len() || b[k] != b'r' {
        return None;
    }
    k += 1;
    let hash_start = k;
    while k < b.len() && b[k] == b'#' {
        k += 1;
    }
    let hashes = k - hash_start;
    if k >= b.len() || b[k] != b'"' {
        return None;
    }
    k += 1;
    while k < b.len() {
        if b[k] == b'"' {
            // Require `hashes` `#` after.
            let mut h = 0;
            while h < hashes && k + 1 + h < b.len() && b[k + 1 + h] == b'#' {
                h += 1;
            }
            if h == hashes {
                return Some(k + hashes);
            }
        }
        k += 1;
    }
    Some(b.len().saturating_sub(1))
}

/// The byte index of the first `;` at bracket depth 0 from `from`.
fn find_semicolon(b: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        if b[i] == b';' {
            return Some(i);
        }
        i += 1;
    }
    None
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
                    && !ALLOW.iter().any(|(f, nd, marker, _)| {
                        path.ends_with(f) && nd == needle && line.contains(marker)
                    })
                    && !PINNED.iter().any(|(f, nd, marker, _)| {
                        path.ends_with(f) && nd == needle && line.contains(marker)
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
}
