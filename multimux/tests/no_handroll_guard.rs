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

/// (file suffix, needle, reason).
const ALLOW: &[(&str, &str, &str)] = &[
    // --- Redaction of a URL the `url` parser REJECTS (spec §9 documented
    // exception): the masking-only fallbacks scan the raw text for the `://`
    // boundary and the `@`. They do NOT reconstruct a secret from the text
    // (behaviourally guarded in-file by
    // `masking_fallbacks_do_not_reconstruct_a_secret`). ---
    (
        "redact.rs",
        "find(\"://\")",
        "redact.rs masking-only fallback for an unparseable URL (spec §9 exception)",
    ),
    // --- SRT query split stays manual (W2b-1 Task 2): a Haivision
    // `streamid=#!::r=...` value contains a `#` that `url`'s `query_pairs()`
    // would cut as a fragment delimiter. Only the AUTHORITY parse moved to the
    // `url` crate; the scheme prefix is stripped here before `split_once('?')`.
    (
        "push/srt.rs",
        "strip_prefix(\"srt://\")",
        "push/srt.rs: the query split stays manual (Haivision streamid `#`)",
    ),
    // --- The RTSP push renders its own ANNOUNCE SDP (`build_sdp`); it moves
    // onto rtsp-runtime's adapter in W2b-2, not here. ---
    (
        "push/rtsp.rs",
        "\"v=0",
        "push/rtsp.rs `build_sdp` renders the ANNOUNCE SDP (W2b-2 moves the RTSP push)",
    ),
];

/// (file suffix, needle, line marker, reason) — like [`ALLOW`] but also
/// requires `line_marker` to appear on the SAME source line, so a whole-file
/// allow is replaced by a line-/fn-pinned one. config.rs's `"v=0` SDP test
/// literals live inside `#[test]` fns whose raw-string braces defeat the
/// `#[cfg(test)] mod tests` skipper; pinning them to their exact lines (not
/// the whole file) still lets any other `"v=0` in config.rs trip the guard.
const PINNED: &[(&str, &str, &str, &str)] = &[
    (
        "config.rs",
        "\"v=0",
        "let sdp = \"v=0\\r\\no=- 0 0 IN IP4",
        "config.rs `parses_json_config_with_rtp_input` test fixture (raw-string braces defeat the module skipper)",
    ),
    (
        "config.rs",
        "\"v=0",
        "let long_sdp = \"v=0\\r\\n\".repeat(50)",
        "config.rs `route_debug_shows_sdp_length_not_full_body` test fixture (raw-string braces defeat the module skipper)",
    ),
    (
        "output/whep.rs",
        "\"v=0",
        "pub const WHEP_TEST_OFFER: &str = \"v=0",
        "output/whep.rs `WHEP_TEST_OFFER` (W2b-1 Task 7 test fixture: a valid SDP offer to admit a real session)",
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
}
