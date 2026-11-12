//! The core never reads a clock: every instant arrives as an argument.
//! **Lexical, not a proof**: a renamed import evades it; review is the control.

use workspace_guards::scan::non_test_source;

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
    "tokio::time",
    "thread::sleep",
];

#[test]
fn no_clock_read_outside_tests() {
    let mut hits = Vec::new();
    for (path, src) in workspace_guards::src_files("smooth-runtime") {
        for (n, line) in non_test_source(&src).lines().enumerate() {
            for needle in NEEDLES {
                if line.contains(needle) {
                    hits.push(format!("{path}:{}: {needle}", n + 1));
                }
            }
        }
    }
    assert!(hits.is_empty(), "clock access in src:\n{}", hits.join("\n"));
}

#[test]
fn the_scanner_bites_on_each_needle() {
    for needle in NEEDLES {
        let src = format!("fn f() {{ let _ = {needle}(); }}\n");
        assert!(non_test_source(&src).contains(needle), "{needle}");
    }
    let in_tests = "#[cfg(test)]\nmod tests {\n    fn f() { let _ = SystemTime::now(); }\n}\n";
    assert!(!non_test_source(in_tests).contains("SystemTime::now"));
}

#[test]
fn the_scan_covers_the_client_module() {
    let files = workspace_guards::src_files("smooth-runtime");
    for name in ["client/engine.rs", "client/stream.rs", "client/output.rs"] {
        assert!(
            files.iter().any(|(p, _)| p.ends_with(name)),
            "{name} is not scanned"
        );
    }
}
