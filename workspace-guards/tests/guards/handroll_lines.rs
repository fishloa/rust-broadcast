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

/// Net `{` minus `}` on a line, ignoring string/char literals and `//` comments.
fn brace_delta(line: &str) -> (i64, bool) {
    let mut delta = 0i64;
    let mut seen_open = false;
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
            // Single-line raw string `r"..."` / `r#"..."#`: skip it whole (a
            // multi-line raw string is not handled).
            'r' if matches!(chars.peek(), Some('"' | '#')) => {
                let mut hashes = 0usize;
                while chars.peek() == Some(&'#') {
                    chars.next();
                    hashes += 1;
                }
                if chars.peek() == Some(&'"') {
                    chars.next();
                    let close: String = core::iter::once('"')
                        .chain(core::iter::repeat_n('#', hashes))
                        .collect();
                    let rest: String = chars.clone().collect();
                    if let Some(end) = rest.find(&close) {
                        for _ in 0..rest[..end + close.len()].chars().count() {
                            chars.next();
                        }
                    }
                }
            }
            '"' => in_str = true,
            '/' if chars.peek() == Some(&'/') => break,
            '\'' => {
                // A char literal ('{', '\n', ...) — skip it; lifetimes have no closing quote.
                let rest: String = chars.clone().take(3).collect();
                if let Some(end) = rest.find('\'') {
                    for _ in 0..=end {
                        chars.next();
                    }
                }
            }
            '{' => {
                delta += 1;
                seen_open = true;
            }
            '}' => delta -= 1,
            _ => {}
        }
    }
    (delta, seen_open)
}

/// Whether the item after the attribute line at `attr` is a `mod`.
fn next_item_is_mod(lines: &[&str], attr: usize) -> Option<usize> {
    let mut j = attr + 1;
    while j < lines.len() && lines[j].trim_start().starts_with("#[") {
        j += 1;
    }
    let t = lines.get(j)?.trim_start();
    let t = t.strip_prefix("pub").map_or(t, |r| {
        let r = r.trim_start();
        if r.starts_with('(') {
            r.find(')').map_or(r, |i| r[i + 1..].trim_start())
        } else {
            r
        }
    });
    t.starts_with("mod ").then_some(j)
}

/// Index of the last line of the brace-matched block that starts on `from`
/// (`mod x;` with no body ends on its own line). The block ends on the line
/// where the running depth returns to zero after a `{` was seen — which may
/// be a later line than the one that opened it.
fn end_of_block(lines: &[&str], from: usize) -> usize {
    let mut depth = 0i64;
    let mut opened = false;
    for (k, line) in lines.iter().enumerate().skip(from) {
        let (d, saw_open) = brace_delta(line);
        depth += d;
        opened |= saw_open;
        if opened && depth <= 0 {
            return k;
        }
        if !opened && line.trim_end().ends_with(';') {
            return k;
        }
    }
    lines.len().saturating_sub(1)
}

/// The production code of a file as `(line number, line)` pairs: comment
/// lines are dropped and ONLY the brace-matched body of a `#[cfg(test)]`
/// (or `#[cfg(all(test, ...))]`) `mod` is skipped.
fn code_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if (t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test"))
            && let Some(m) = next_item_is_mod(&lines, i)
        {
            i = end_of_block(&lines, m) + 1;
            continue;
        }
        if !t.starts_with("//") {
            out.push((i + 1, lines[i]));
        }
        i += 1;
    }
    out
}

fn hits_in(c: &Crate, path: &str, text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for (line_no, line) in code_lines(text) {
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
               #[cfg(all(test, feature = \"x\"))]\n\
               mod more {\n\
               fn h() {\n\
               std::thread::sleep(d)\n\
               }\n\
               }\n\
               fn after() { std::thread::sleep(d); }\n";
    let hits = hits_in(&synthetic(), "src/synthetic.rs", src);
    assert!(
        hits.iter().any(|h| h.contains("src/synthetic.rs:1:")),
        "the offender on line 1 must be reported: {hits:?}"
    );
    assert!(
        hits.iter().any(|h| h.contains("synthetic.rs:15:")),
        "code after a multi-line test module must still be scanned: {hits:?}"
    );
    assert!(
        hits.iter()
            .all(|h| h.contains("synthetic.rs:1:") || h.contains("synthetic.rs:15:")),
        "comment and test-module bodies must not be reported: {hits:?}"
    );
}
