//! The DASH client path never reads a clock: every instant is an argument (`now_us`,
//! `set_wall_clock`). Bans the wall-clock and monotonic readers outside `#[cfg(test)]` in the
//! source of `broadcast-client` AND `dash-runtime`, so one guard covers the whole DASH path.
//!
//! **Lexical, not a proof**: a renamed import evades it; review is the control.

use workspace_guards::scan::{non_test_source, out_of_line_test_modules};
use workspace_guards::src_files;

const NEEDLES: &[&str] = &[
    "SystemTime::now",
    "Instant::now",
    "Timestamp::now",
    "Zoned::now",
    "UNIX_EPOCH",
    "std::time::SystemTime",
    "std::time::Instant",
    "Utc::now",
    "Local::now",
    "OffsetDateTime::now",
    "tokio::time",
];

fn scanned_files() -> Vec<(String, String)> {
    ["broadcast-client", "dash-runtime"]
        .into_iter()
        .flat_map(|name| {
            src_files(name)
                .into_iter()
                .map(move |(rel, text)| (format!("{name}/{rel}"), text))
        })
        .collect()
}

/// Not on the DASH path, so outside this guard's scope by design: the RTSP
/// pull takes one `Instant` epoch at construction so the shared
/// `rtsp_runtime::ClientSession` keepalive schedule can be converted to and
/// from its caller's `now_us` — an `Instant` cannot be built from a
/// caller-supplied `u64` any other way. It uses that epoch only for the
/// RTSP session timeout (RFC 2326 §12.37), never for an HTTP fetch, a
/// retry, a backoff or any other DASH decision.
const OUT_OF_SCOPE: &[&str] = &["broadcast-client/src/source/rtsp.rs"];

fn hits_in(files: &[(String, String)]) -> Vec<String> {
    let skipped = out_of_line_test_modules(files);
    let mut out = Vec::new();
    for (path, src) in files {
        if skipped.contains(path) {
            continue;
        }
        if OUT_OF_SCOPE.iter().any(|scope| path == scope) {
            continue;
        }
        for (n, line) in non_test_source(src).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for needle in NEEDLES {
                if line.contains(needle) {
                    out.push(format!("{path}:{}: {needle}", n + 1));
                }
            }
        }
    }
    out
}

#[test]
fn no_clock_read_outside_tests_in_the_dash_path() {
    let hits = hits_in(&scanned_files());
    assert!(hits.is_empty(), "clock access in src:\n{}", hits.join("\n"));
}

#[test]
fn both_crates_are_scanned() {
    let files = scanned_files();
    for suffix in [
        "broadcast-client/src/source/dash.rs",
        "dash-runtime/src/client/engine.rs",
    ] {
        assert!(
            files.iter().any(|(p, _)| p.contains(suffix)),
            "{suffix} is not scanned"
        );
    }
}

#[test]
fn the_scanner_bites_on_each_needle() {
    for needle in NEEDLES {
        let files = vec![(
            "synthetic.rs".to_owned(),
            format!("fn f() {{ let _ = {needle}(); }}\n"),
        )];
        assert_eq!(hits_in(&files).len(), 1, "{needle}");
    }
}

#[test]
fn a_test_module_body_is_exempt_but_the_next_item_is_not() {
    let src = "#[cfg(test)]\nmod tests {\n    fn f() { let _ = Instant::now(); }\n}\nfn real() { let _ = Instant::now(); }\n";
    let files = vec![("synthetic.rs".to_owned(), src.to_owned())];
    assert_eq!(hits_in(&files).len(), 1);
}
