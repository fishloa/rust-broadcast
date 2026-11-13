//! Tripwire against hand-rolled generic protocol code coming back (W2a spec §5).
//!
//! Scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies. Cross-references
//! `tests/it/harness_guard.rs` (W2b-1 Task 10), which guards THIS crate's test
//! harness (reserve-then-rebind loops, sleeps) — this file guards `src/`
//! only.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper or a string assembled from pieces
//! evades it. Code review is the real control.
//!
//! The `#[cfg(test)] mod` cut is `workspace_guards::scan::non_test_source`
//! (its string / raw-string / comment / non-ASCII cases are tested there). The
//! needles themselves are still lexical (a tokeniser built on `split(',')` is
//! not detected), and reported line numbers are of the stripped text.

use std::fs;
use std::path::Path;
use workspace_guards::scan::non_test_source;

/// A line-pinned allow: `(file suffix, needle, line marker, reason)`. The
/// needle is only allowed on a source line that ALSO contains `line_marker`, so
/// a new occurrence of the same needle elsewhere in the same file still fails
/// (the previous file+needle allowlist let any new `find("://")` in `redact.rs`
/// through).
const ALLOW: &[(&str, &str, &str, &str)] = &[
    // --- Redaction of a URL the `url` parser REJECTS (spec §9.5 documented
    // exception): the masking-only fallbacks scan the raw text for the `://`
    // boundary and the `@`. They do NOT reconstruct a secret from the text
    // (behaviourally guarded in-file by
    // `masking_fallbacks_do_not_reconstruct_a_secret`). The two `raw.find("://")`
    // sites are `redact_unparseable_userinfo` / `redact_unparseable_destination`.
    (
        "redact.rs",
        "find(\"://\")",
        "let Some(scheme_end) = raw.find(\"://\") else {",
        "redact.rs masking-only `://` boundary scan for an unparseable URL (spec §9.5)",
    ),
    (
        "redact.rs",
        "find(\"://\")",
        "if let Some(scheme_end) = url.find(\"://\") {",
        "redact.rs `scrub_destination_secrets`: the `://` boundary of the URL being scrubbed from an error message (spec §9.5)",
    ),
    (
        "redact.rs",
        "rfind('@')",
        "let Some(at) = authority.rfind('@') else {",
        "redact.rs masking-only fallback: locate the credential's `@` boundary (spec §9.5)",
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
    // --- SRT query split stays manual (spec §9.6 documented exception): a
    // Haivision `streamid=#!::r=...` value contains an unencoded `#` that
    // `url`'s `query_pairs()` cuts as a fragment delimiter (verified against
    // url 2.5.8). Only the AUTHORITY parse moved to the `url` crate; the scheme
    // prefix is stripped here before `split_once('?')`.
    (
        "push/srt.rs",
        "strip_prefix(\"srt://\")",
        "let stripped = url.strip_prefix(\"srt://\").unwrap_or(url);",
        "push/srt.rs: the query split stays manual (Haivision streamid `#`, spec §9.6)",
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

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let files = workspace_guards::src_files("multimux");
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

/// Calls that read a machine clock. `dash_pull` takes every instant from
/// `crate::clock::HostClock` (design W-f: no `Timestamp::now` or
/// `SystemTime::now` in the pull path), so none of these may appear in its
/// production code; the clock seam itself lives in `src/clock.rs`.
const CLOCK_READS: &[&str] = &[
    "Instant::now",
    "SystemTime::now",
    "Timestamp::now",
    "UNIX_EPOCH",
    ".elapsed()",
];

/// The files of `dir` that are compiled outside `#[cfg(test)]`: every `.rs`
/// file except those `mod.rs` declares as `#[cfg(test)] mod name;`.
fn production_files(dir: &Path) -> Vec<(String, String)> {
    let all: Vec<(String, String)> = workspace_guards::rs_files(dir)
        .into_iter()
        .map(|p| {
            let text = fs::read_to_string(&p).expect("read source");
            (p.display().to_string(), text)
        })
        .collect();
    let declaring = fs::read_to_string(dir.join("mod.rs")).expect("mod.rs");
    let test_only: Vec<String> = declaring
        .lines()
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| w[0].trim() == "#[cfg(test)]")
        .filter_map(|w| {
            w[1].trim()
                .strip_prefix("mod ")?
                .strip_suffix(';')
                .map(str::to_owned)
        })
        .collect();
    all.into_iter()
        .filter(|(path, _)| {
            let stem = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            !test_only.iter().any(|t| t == stem)
        })
        .collect()
}

fn clock_reads_in(files: &[(String, String)]) -> Vec<String> {
    let mut hits = Vec::new();
    for (path, src) in files {
        for (n, line) in non_test_source(src).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for needle in CLOCK_READS {
                if line.contains(needle) {
                    hits.push(format!("{path}:{}: {needle}", n + 1));
                }
            }
        }
    }
    hits
}

#[test]
fn dash_pull_reads_no_clock_outside_tests() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/source/dash_pull");
    let files = production_files(&dir);
    assert!(
        !files.is_empty(),
        "the production files of dash_pull were not found: {:?}",
        files.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
    let hits = clock_reads_in(&files);
    assert!(
        hits.is_empty(),
        "dash_pull must take time from crate::clock::HostClock, not read it:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_clock_guard_bites_on_production_code_only() {
    let leaky = vec![(
        "dash_pull/x.rs".to_owned(),
        "fn a() { let t = std::time::Instant::now(); }\n#[cfg(test)]\nmod tests {\n    fn t() { let _ = std::time::SystemTime::now(); }\n}\n".to_owned(),
    )];
    let hits = clock_reads_in(&leaky);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].contains("Instant::now"));
    for needle in CLOCK_READS {
        let one = vec![("f.rs".to_owned(), format!("fn a() {{ {needle}; }}\n"))];
        assert_eq!(clock_reads_in(&one).len(), 1, "{needle} must be caught");
    }
}
