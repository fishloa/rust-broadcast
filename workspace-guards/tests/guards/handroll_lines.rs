//! Tripwire (de-hand-roll W1-P, spec §5), shape 2: fails if a table crate's `src/` (outside
//! `#[cfg(test)] mod` bodies) reintroduces a hand-rolled pattern the
//! migration removed — poison-recovering std locks, sleep-based waits, HTTP framing literals, calendar maths, hand-built SDP.
//! **This is a lexical tripwire, not a proof**; a renamed helper or a split
//! literal evades it. Code review is the real control. Every allowlist entry
//! carries a reason.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// (pattern, why it is banned).
    pub patterns: &'static [(&'static str, &'static str)],
    /// (file suffix, pattern, text the offending LINE must contain, reason).
    /// Pinned to a line, not a whole file, so a new offender in the same file
    /// still trips. Each reason must be non-empty.
    pub allow: &'static [(&'static str, &'static str, &'static str, &'static str)],
}

/// The production code of a file as `(line number, line)` pairs: comment lines
/// are dropped and every test-only item (`#[cfg(test)]`, `#[cfg(all(test,
/// ..))]`) is blanked with the shared lexer, so production code after a
/// mid-file test module is still scanned.
fn code_lines(text: &str) -> Vec<(usize, String)> {
    workspace_guards::scan::without_attributed_items(
        text,
        workspace_guards::scan::test_only_attribute,
    )
    .lines()
    .enumerate()
    .filter(|(_, line)| !line.trim_start().starts_with("//"))
    .map(|(no, line)| (no + 1, line.to_string()))
    .collect()
}

fn hits_in(c: &Crate, path: &str, text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for (line_no, line) in code_lines(text) {
        let line = line.as_str();
        for (pat, why) in c.patterns {
            if line.contains(pat)
                && !c.allow.iter().any(|(f, p, l, r)| {
                    path.ends_with(f) && p == pat && line.contains(l) && !r.is_empty()
                })
            {
                hits.push(format!("{path}:{line_no}: `{pat}` — {why}"));
            }
        }
    }
    hits
}

#[test]
fn no_handrolled_patterns_in_production_source() {
    let mut hits = Vec::new();
    for c in crate::handroll_data::LINE_PATTERNS {
        let files = src_files(c.name);
        assert!(
            !files.is_empty(),
            "{}: guard found no source files — path bug",
            c.name
        );
        for (path, text) in &files {
            hits.extend(hits_in(c, &format!("{}/{path}", c.name), text));
        }
    }
    assert!(
        hits.is_empty(),
        "hand-rolled patterns reintroduced:\n{}",
        hits.join("\n")
    );
}

#[test]
fn every_allowlist_entry_has_a_reason() {
    for c in crate::handroll_data::LINE_PATTERNS {
        for (file, pat, line, reason) in c.allow {
            assert!(
                !reason.trim().is_empty() && !line.is_empty(),
                "{}: allowlist entry ({file}, {pat}) needs a line pin and a reason",
                c.name
            );
        }
    }
}

fn synthetic() -> Crate {
    Crate {
        name: "synthetic",
        patterns: &[
            ("thread::sleep", "sleep-based wait"),
            ("sleep(", "sleep-based wait"),
        ],
        allow: &[],
    }
}

/// The guard must bite: a synthetic offender is detected, a comment or a
/// `#[cfg(test)] mod` body (plain and `all(test, ..)` forms) is not.
#[test]
fn raw_string_braces_do_not_derail_block_matching() {
    let src = "#[cfg(test)]\nmod tests {\nconst S: &str = r#\"x\"{\"#\n}\nfn after() { std::thread::sleep(d); }\n";
    let hits = hits_in(&synthetic(), "src/raw.rs", src);
    assert!(hits.iter().any(|h| h.contains("raw.rs:5:")), "{hits:?}");
}

#[test]
fn guard_detects_a_synthetic_offender() {
    let src = "fn f() { std::thread::sleep(d); }\n\
               // thread::sleep in a comment\n\
               #[cfg(test)]\n\
               mod tests {\n\
               fn g() {\n\
               std::thread::sleep(d)\n\
               }\n\
               }\n\
               #[cfg(all(test, unix))]\n\
               mod more {\n\
               fn h() {\n\
               std::thread::sleep(d)\n\
               }\n\
               }\n\
               fn after() { std::thread::sleep(d); }\n";
    let hits = hits_in(&synthetic(), "src/synthetic.rs", src);
    // A single-line test module and an `all(test, unix)` one are both
    // test-only; the module body's line numbers are checked by their own
    // tests below, which do not feed one module's braces into another.
    assert!(
        hits.iter().any(|h| h.contains("src/synthetic.rs:1:")),
        "the offender on line 1 must be reported: {hits:?}"
    );
    assert!(
        hits.iter().any(|h| h.contains("synthetic.rs:15:")),
        "code after a test-only module must still be scanned: {hits:?}"
    );
}

#[test]
fn a_test_module_in_the_middle_does_not_hide_the_code_after_it() {
    let src = "#[cfg(test)]\nmod t {\n    fn f() {}\n}\nfn g() {\n    std::thread::sleep(d);\n}\n";
    let hits = hits_in(&synthetic(), "src/mid.rs", src);
    assert!(hits.iter().any(|h| h.contains("mid.rs:6:")), "{hits:?}");
}

#[test]
fn a_test_only_all_cfg_module_is_not_production() {
    let src = "#[cfg(all(test, unix))]\nmod more {\n    fn h() {\n        std::thread::sleep(d);\n    }\n}\n";
    let hits = hits_in(&synthetic(), "src/all.rs", src);
    assert!(
        hits.is_empty(),
        "a test-only module is not production: {hits:?}"
    );
}

#[test]
fn a_condition_that_is_not_test_only_is_production() {
    let src = "#[cfg(any(test, feature = \"x\"))]\nmod more {\n    fn h() {\n        std::thread::sleep(d);\n    }\n}\n";
    let hits = hits_in(&synthetic(), "src/any.rs", src);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().all(|h| h.contains("src/any.rs:4:")), "{hits:?}");
}
