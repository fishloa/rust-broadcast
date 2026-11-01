//! The core never reads a clock: every instant arrives as an argument
//! (`docs/dash-runtime/DESIGN.md` 3.1). Bans the wall-clock and monotonic
//! readers outside `#[cfg(test)]`.
//!
//! **Lexical, not a proof**: a renamed import evades it; review is the control.

use crate::common;

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

#[test]
fn no_clock_read_outside_tests() {
    let hits = common::scan::hits(NEEDLES, &[]);
    assert!(hits.is_empty(), "clock access in src:\n{}", hits.join("\n"));
}

#[test]
fn the_scanner_bites_on_each_needle() {
    for needle in NEEDLES {
        let src = format!("fn f() {{ let _ = {needle}(); }}\n");
        let hit = common::scan::non_test_source(&src).contains(needle);
        assert!(hit, "{needle}");
    }
    let in_tests = "#[cfg(test)]\nmod tests {\n    fn f() { let _ = SystemTime::now(); }\n}\n";
    assert!(!common::scan::non_test_source(in_tests).contains("SystemTime::now"));
}

#[test]
fn the_scan_covers_the_client_module() {
    let files = common::scan::src_files();
    for name in ["client/engine.rs", "client/action.rs", "client/output.rs"] {
        assert!(
            files.iter().any(|(path, _)| path.ends_with(name)),
            "{name} is not scanned"
        );
    }
}
