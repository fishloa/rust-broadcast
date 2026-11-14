//! Guard: a host-clock deadline is `Timestamp::checked_add(..)`, never a
//! saturating one. `docs/API-CONVENTIONS.md` (Time) says `None` means the timer
//! never fires and a saturated deadline reads as due now forever, so production
//! code in the crates that compute deadlines from `broadcast_common::stage::Timestamp`
//! must not combine a `Timestamp` with a span by saturation. Scans every
//! `src/**/*.rs` outside `#[cfg(test)]` items and fails on:
//!
//! - `saturating_add` / `saturating_add_nanos` with a `Timestamp` operand
//!   (`now.saturating_add(span)`, `Timestamp::from_micros(at).saturating_add(..)`,
//!   `t.saturating_add(Duration::..)`, `.saturating_add(TS_WAKE_TICK)`), and
//! - `saturating_duration_since(timestamp)` on a span (a span clamps instead of
//!   asking whether it already passed).
//!
//! Arithmetic on a plain `u64` microsecond reading is out of this guard's reach
//! (it cannot tell one `u64` from another), which is what the migration of each
//! crate to `Timestamp` end to end fixes.
//!
//! **This is a lexical tripwire, not a proof.** An operand renamed to something
//! that does not read as a `Timestamp` evades it; code review is the real
//! control. Every `ALLOW` entry is line-pinned and states why the hit is not a
//! deadline; a stale entry (no longer matching) fails.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// `(path suffix, line number, needle, reason)` — reviewed, line-pinned
    /// exceptions.
    pub allow: &'static [(&'static str, usize, &'static str, &'static str)],
}

/// The needles, and for each the shape of operand that makes it a deadline.
const SATURATING_ADD: &str = "saturating_add";
const SATURATING_ADD_NANOS: &str = "saturating_add_nanos";

/// What follows [`SATURATING_ADD`] in [`SATURATING_ADD_NANOS`].
const NANOS_SUFFIX: &str = "_nanos";

const SATURATING_DURATION_SINCE: &str = "saturating_duration_since";

/// What reads as a `Timestamp`: the shared clock type and the `*_at` names a
/// host clock instant is bound to (`open_at`, `last_byte_at`, `next_notice_at`).
/// A bare `now`, `at` or `due` is deliberately absent: a match binding or a
/// string prefix does not say what type it carries, so pinning those would need
/// an allowlist line per match arm.
const CLOCK_OPERANDS: &[&str] = &["Timestamp", "_at"];

/// The production code of a file as `(line number, line)` pairs, from the shared
/// lexer so a brace or quote inside a string cannot desynchronise it.
fn code_lines(text: &str) -> Vec<(usize, String)> {
    workspace_guards::scan::non_test_source_blanked(text)
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .map(|(no, line)| (no + 1, line.to_string()))
        .collect()
}

/// One line of a source fragment as the lexer presents it: a fragment without a
/// line break is read as the final line of a file, which is why a trailing newline
/// is added before scanning.
fn code_line(text: &str) -> String {
    let mut src = text.to_string();
    if !src.ends_with('\n') {
        src.push('\n');
    }
    code_lines(&src)
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The argument text of a call whose `(` is already consumed: up to its
/// matching `)`.
fn argument(after: &str) -> &str {
    let mut depth = 0usize;
    for (i, c) in after.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return &after[..i];
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    after
}

/// Whether `before` ends with one of `suffixes`, ignoring trailing whitespace.
fn prefix_ends_with(before: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|s| before.trim_end().ends_with(s))
}

fn reads_as_clock(expr: &str) -> bool {
    CLOCK_OPERANDS.iter().any(|needle| expr.contains(needle))
}

/// A span type in the argument: a `Duration` or a named span constant.
fn carries_a_span(expr: &str) -> bool {
    expr.contains("Duration") || expr.contains("_TICK") || expr.contains("_SPAN")
}

/// Whether `line` computes a `Timestamp` deadline by saturation.
fn saturates_a_deadline(line: &str) -> bool {
    for (i, _) in line.match_indices(SATURATING_DURATION_SINCE) {
        let elapsed = line.contains("duration.saturating_duration_since")
            || line.contains("span.saturating_duration_since")
            || prefix_ends_with(&line[..i], &["duration", "span"]);
        if elapsed
            && reads_as_clock(argument(
                line[i + SATURATING_DURATION_SINCE.len()..].trim_start(),
            ))
        {
            return true;
        }
    }
    line.match_indices(SATURATING_ADD).any(|(i, _)| {
        let after = &line[i + SATURATING_ADD.len()..];
        let Some(opening) = after.find('(') else {
            return false;
        };
        if !after[..opening].trim().is_empty() {
            return false;
        }
        let arg = argument(&after[opening + 1..]).trim();
        // A clock-typed receiver or operand, a `Duration`/tick span operand, or the
        // nanosecond form is a deadline: a plain counter adds only counts.
        reads_as_clock(line)
            || reads_as_clock(arg)
            || carries_a_span(arg)
            || (after.starts_with(NANOS_SUFFIX) && arg.contains("nanos"))
    })
}

fn problems_in(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut hits = Vec::new();
    let mut used = vec![false; c.allow.len()];
    for (rel, text) in files {
        for (no, line) in code_lines(text) {
            if !saturates_a_deadline(&line) {
                continue;
            }
            let needle = [
                SATURATING_DURATION_SINCE,
                SATURATING_ADD_NANOS,
                SATURATING_ADD,
            ]
            .into_iter()
            .find(|n| line.contains(n))
            .unwrap_or(SATURATING_ADD);
            if let Some(i) = c
                .allow
                .iter()
                .position(|(s, l, n, _)| rel.ends_with(s) && *l == no && *n == needle)
            {
                used[i] = true;
            } else {
                hits.push(format!(
                    "{}: {rel}:{no}: a deadline is Timestamp::checked_add (None = never fires), never a saturating one: {}",
                    c.name,
                    line.trim()
                ));
            }
        }
    }
    for (a, u) in c.allow.iter().zip(&used) {
        if !*u {
            hits.push(format!("{}: stale ALLOW entry: ({}, {})", c.name, a.0, a.1));
        }
    }
    hits
}

#[test]
fn no_saturated_timestamp_deadline_in_src() {
    let mut hits = Vec::new();
    for c in crate::deadlines_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        hits.extend(problems_in(c, &files));
    }
    assert!(
        hits.is_empty(),
        "saturated deadline(s) (use Timestamp::checked_add, or add a reasoned line-pinned ALLOW entry in deadlines_data.rs):\n{}",
        hits.join("\n")
    );
}

#[test]
fn table_names_are_workspace_members() {
    let names: Vec<&str> = crate::deadlines_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    crate::common::assert_names_are_members("deadlines", &names);
}

#[test]
fn scanner_catches_deadline_shapes_and_skips_test_code_and_plain_counters() {
    let hits: Vec<&str> = [
        "let at = now.saturating_add(span);",
        "let at = one.saturating_add(TS_WAKE_TICK);",
        "let at = t.saturating_add(Duration::from_millis(50));",
        "let at = now.saturating_add(TS_WAKE_TICK);",
        "let over = duration.saturating_duration_since(open_at);",
    ]
    .into_iter()
    .filter(|src| saturates_a_deadline(&code_line(src)))
    .collect();
    assert_eq!(hits.len(), 4, "{hits:?}");
    for line in [
        "self.dropped = self.dropped.saturating_add(1);",
        "self.attempts = self.attempts.saturating_add(1);",
        "let micros = span.saturating_add(other_span);",
        "// now.saturating_add(span)",
        "#[cfg(test)]\nmod tests {\n    fn f(now: Timestamp) { let _ = now.saturating_add(span); }\n}",
        "now.saturating_add_nanos(1);",
    ] {
        assert!(!saturates_a_deadline(&code_line(line)), "{line}");
    }
}

#[test]
fn allow_pin_and_stale_entry_are_enforced() {
    let src = vec![(
        "src/x.rs".to_string(),
        "fn f(now: Timestamp) { let _ = now.saturating_add(span); }\n".to_string(),
    )];
    let pinned = Crate {
        name: "synthetic",
        allow: &[("src/x.rs", 1, SATURATING_ADD, "reason")],
    };
    assert!(problems_in(&pinned, &src).is_empty());
    let wrong_line = Crate {
        name: "synthetic",
        allow: &[("src/x.rs", 9, SATURATING_ADD, "reason")],
    };
    let hits = problems_in(&wrong_line, &src);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits[0].starts_with("synthetic: src/x.rs:1:"), "{hits:?}");
}
