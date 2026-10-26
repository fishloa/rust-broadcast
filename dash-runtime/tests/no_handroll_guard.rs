//! Tripwire against hand-rolled generic protocol code (de-hand-roll spec 5).
//!
//! Scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies for HTTP status-line
//! literals, a CRLFCRLF terminator, `find("://")`, `strip_prefix("<scheme>://")`,
//! SDP line building, civil-date helpers, a base64 alphabet literal and
//! `thread::sleep` / `sleep(`.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper or a string
//! assembled from pieces evades it. Code review is the real control.

mod common;

const ALLOW: &[(&str, &str, &str)] = &[];

const NEEDLES: &[&str] = &[
    "\"HTTP/1.",
    "\\r\\n\\r\\n",
    "find(\"://\")",
    "strip_prefix(\"http://\")",
    "strip_prefix(\"https://\")",
    "\"a=",
    "\"m=",
    "\"v=0",
    "civil_from_days",
    "days_from_civil",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    "thread::sleep",
    "sleep(",
];

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let hits = common::scan::hits(NEEDLES, ALLOW);
    assert!(
        hits.is_empty(),
        "hand-rolled protocol code found (allowlist with a reason, or use the crate):\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_scanner_itself_bites() {
    let src = "fn real() { let x = \"HTTP/1.1 200\"; }\n#[cfg(test)]\nmod tests {\n    fn t() { let y = \"HTTP/1.1 200\"; }\n}\n";
    assert_eq!(
        common::scan::non_test_source(src)
            .matches("HTTP/1.")
            .count(),
        1
    );
    let out_of_line = "#[cfg(test)]\nmod tests;\nfn real() { let x = \"HTTP/1.1 200\"; }\n";
    assert_eq!(
        common::scan::non_test_source(out_of_line)
            .matches("HTTP/1.")
            .count(),
        1
    );
}
