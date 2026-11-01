//! The DASH client path never reads a clock: every instant is an argument (`now_us`,
//! `set_wall_clock`). Bans the wall-clock and monotonic readers outside `#[cfg(test)]` in the
//! source of `broadcast-client` AND `dash-runtime`, so one guard covers the whole DASH path.
//!
//! **Lexical, not a proof**: a renamed import evades it; review is the control.

#[path = "../../../dash-runtime/tests/it/common/scan.rs"]
#[allow(dead_code)]
mod scan;

use std::path::{Path, PathBuf};

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

fn crate_src(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(name)
        .join("src")
}

fn scanned_files() -> Vec<(String, String)> {
    let mut files = Vec::new();
    for name in ["broadcast-client", "dash-runtime"] {
        scan::rs_files(&crate_src(name), &mut files);
    }
    files
}

fn out_of_line_test_files(files: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (path, src) in files {
        let p = Path::new(path);
        let dir = if p.file_name().is_some_and(|n| n == "mod.rs") {
            p.parent().map(Path::to_path_buf)
        } else {
            p.parent().zip(p.file_stem()).map(|(d, s)| d.join(s))
        };
        let Some(dir) = dir else { continue };
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.trim_start().starts_with("#[cfg(test)]") {
                continue;
            }
            if let Some(next) = lines.get(i + 1)
                && let Some(name) = next
                    .trim()
                    .trim_start_matches("pub ")
                    .strip_prefix("mod ")
                    .and_then(|r| r.strip_suffix(';'))
            {
                out.push(dir.join(format!("{name}.rs")).display().to_string());
            }
        }
    }
    out
}

fn hits_in(files: &[(String, String)]) -> Vec<String> {
    let skipped = out_of_line_test_files(files);
    let mut out = Vec::new();
    for (path, src) in files {
        if skipped.contains(path) {
            continue;
        }
        for (n, line) in scan::non_test_source(src).lines().enumerate() {
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
