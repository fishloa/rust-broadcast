//! Pins `check_dash_mpd`'s findings on every committed MPD fixture (#2340).
//!
//! The validator's MPD reader moves from transmux to dash-runtime; the move is
//! safe only if every fixture yields the same `fixture<TAB>rule_id<TAB>severity`
//! lines as before; a fixture with no findings is recorded as `clean`, so a
//! reader that rejected it would show up here. Without a private checkout only
//! the public rows are compared.
#![cfg(feature = "std")]

use std::fmt::Write as _;
use std::path::Path;

use media_doctor::{Report, check_dash_mpd};

const GOLDEN: &str = "golden/media-doctor/dash-validator-findings.txt";
const PUBLIC_PREFIX: &str = "fixtures/";
const PRIVATE_PREFIX: &str = "private/";
const REGENERATE: &str = "REGENERATE_DASH_VALIDATOR_GOLDEN";
const CLEAN: &str = "clean";
const ISO_EXAMPLES: &str = "specs/iso_iec_23009-1_dash_2026_ed6_schema";

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "mpd") {
            out.push(path);
        }
    }
}

fn corpus() -> Vec<(String, std::path::PathBuf)> {
    let public_root = test_fixtures::root();
    let mut public = Vec::new();
    walk(&public_root, &mut public);
    let mut out: Vec<(String, std::path::PathBuf)> = public
        .into_iter()
        .filter_map(|p| {
            let rel = p
                .strip_prefix(&public_root)
                .ok()?
                .to_string_lossy()
                .into_owned();
            (!rel.starts_with("golden/")).then(|| (format!("{PUBLIC_PREFIX}{rel}"), p))
        })
        .collect();
    if let Some(private_root) = test_fixtures::private_root() {
        let mut private = test_fixtures::private_files("", &["mpd"]);
        let examples = private_root.join(ISO_EXAMPLES);
        assert!(
            examples.is_dir(),
            "private checkout present but {} is missing",
            examples.display()
        );
        walk(&examples, &mut private);
        for p in private {
            let rel = p
                .strip_prefix(&private_root)
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned();
            out.push((format!("{PRIVATE_PREFIX}{rel}"), p));
        }
    }
    out.sort();
    out
}

fn findings() -> String {
    let mut out = String::new();
    for (name, path) in corpus() {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut report = Report::new();
        check_dash_mpd(&text, &mut report);
        let mut lines: Vec<String> = report
            .findings()
            .iter()
            .map(|f| format!("{name}\t{}\t{}", f.rule_id, f.severity.name()))
            .collect();
        lines.sort();
        if lines.is_empty() {
            lines.push(format!("{name}\t{CLEAN}"));
        }
        for line in lines {
            let _ = writeln!(out, "{line}");
        }
    }
    out
}

#[test]
fn findings_match_the_golden_corpus() {
    let got = findings();
    if std::env::var_os(REGENERATE).is_some() {
        std::fs::write(test_fixtures::path(GOLDEN), &got).expect("write golden");
        return;
    }
    let want = test_fixtures::read_to_string(GOLDEN);
    let have_private = test_fixtures::private_root().is_some();
    let keep = |l: &&str| have_private || l.starts_with(PUBLIC_PREFIX);
    let want: Vec<&str> = want.lines().filter(keep).collect();
    let got: Vec<&str> = got.lines().filter(keep).collect();
    assert!(
        !want.is_empty(),
        "golden has no findings; regenerate it with {REGENERATE}=1"
    );
    assert_eq!(
        got, want,
        "DASH validator findings changed on the fixture corpus"
    );
}
