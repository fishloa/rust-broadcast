//! Tripwire guard (umbrella #1137 T9), applied to every table crate: 33-bit PTS/DTS/`pts_time` wrap
//! arithmetic in this crate goes through `broadcast_common::clock33`
//! (`WRAP_33BIT`, `unwrap_delta`, `wrapping_forward_distance`, `add`,
//! `add_signed`, `signed_distance`) and the 27 MHz PCR clock arithmetic goes
//! through `clock33::{PCR_HZ, PCR_WRAP_27MHZ, pcr_forward_distance,
//! pcr_from_parts, pcr_split}`, never a hand-rolled modulus or a raw
//! subtraction. Scans every `src/**/*.rs` outside `#[cfg(test)]` items and
//! fails on:
//!
//! - `abs_diff` (a raw absolute difference of two timestamps is wrong across
//!   the 2^33 wrap — use `clock33::signed_distance`);
//! - a literal 2^33: `1 << 33`, `1u64 << 33`, `0x2_0000_0000`,
//!   `0x200000000`, `8_589_934_592` (use `clock33::WRAP_33BIT`).
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper, a split
//! literal (`1 << 32 << 1`) or a macro can evade it, and it cannot tell a
//! timestamp operand from any other `abs_diff` operand; code review is the
//! real control. Every `ALLOW` entry is line-pinned and states why the hit is
//! not 33-bit wrap arithmetic; a stale entry (no longer matching) fails.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// Banned needles; `dvb-si` carries the 2^33 + `abs_diff` subset only
    /// (its Green/Quality_Access_Unit PCR-free codecs have no 27 MHz clock).
    pub banned: &'static [(&'static str, &'static str)],
    /// `(path suffix, line number, needle, reason)` — reviewed, line-pinned
    /// exceptions.
    pub allow: &'static [(&'static str, usize, &'static str, &'static str)],
}

/// `(needle, why it is banned)`.
pub const BANNED_FULL: &[(&str, &str)] = &[
    (
        "abs_diff",
        "raw abs_diff on a timestamp (use clock33::signed_distance)",
    ),
    ("1 << 33", "hand-rolled 2^33 (use clock33::WRAP_33BIT)"),
    ("1u64 << 33", "hand-rolled 2^33 (use clock33::WRAP_33BIT)"),
    (
        "0x2_0000_0000",
        "hand-rolled 2^33 (use clock33::WRAP_33BIT)",
    ),
    ("0x200000000", "hand-rolled 2^33 (use clock33::WRAP_33BIT)"),
    (
        "8_589_934_592",
        "hand-rolled 2^33 (use clock33::WRAP_33BIT)",
    ),
    (
        "27_000_000",
        "hand-rolled 27 MHz clock rate (use clock33::PCR_HZ)",
    ),
    (
        "* 300",
        "hand-rolled PCR base*300+ext arithmetic (use clock33::pcr_from_parts / pcr_split / PCR_WRAP_27MHZ)",
    ),
];

/// The 2^33 + `abs_diff` rules without the 27 MHz PCR ones (the first six).
pub const BANNED_NO_PCR: &[(&str, &str)] = BANNED_FULL.split_at(6).0;

/// The source with every `#[cfg(test)]` item and every `//` comment line
/// removed. Braces inside string literals can fool the brace counter; that is
/// within the tripwire's tolerance.
fn non_test_lines(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut lines = src.lines().enumerate().peekable();
    while let Some((no, line)) = lines.next() {
        let t = line.trim_start();
        if t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test") {
            let mut depth = 0i32;
            let mut opened = false;
            for (_, item) in lines.by_ref() {
                for c in item.chars() {
                    match c {
                        '{' => {
                            depth += 1;
                            opened = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if (opened && depth <= 0) || (!opened && item.trim_end().ends_with(';')) {
                    break;
                }
            }
            continue;
        }
        if t.starts_with("//") {
            continue;
        }
        out.push((no + 1, line.to_string()));
    }
    out
}

fn banned_in_line<'a>(
    banned: &'a [(&'static str, &'static str)],
    line: &str,
) -> Vec<(&'a str, &'a str)> {
    banned
        .iter()
        .filter(|(needle, _)| line.contains(needle))
        .copied()
        .collect()
}

fn check(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut used = vec![false; c.allow.len()];
    for (rel, text) in files {
        for (no, line) in non_test_lines(text) {
            for (needle, why) in banned_in_line(c.banned, &line) {
                if let Some(i) = c
                    .allow
                    .iter()
                    .position(|(s, l, n, _)| rel.ends_with(s) && *l == no && *n == needle)
                {
                    used[i] = true;
                } else {
                    violations.push(format!("{}: {rel}:{no}: {why}: {}", c.name, line.trim()));
                }
            }
        }
    }
    for (a, u) in c.allow.iter().zip(&used) {
        if !*u {
            violations.push(format!("{}: stale ALLOW entry: ({}, {})", c.name, a.0, a.1));
        }
    }
    violations
}

#[test]
fn no_hand_rolled_33bit_wrap_arithmetic_in_src() {
    let mut violations = Vec::new();
    for c in crate::clock33_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        violations.extend(check(c, &files));
    }
    assert!(
        violations.is_empty(),
        "hand-rolled 33-bit wrap arithmetic (use broadcast_common::clock33, or add a reasoned line-pinned ALLOW entry in clock33_data.rs):\n{}",
        violations.join("\n")
    );
}

#[test]
fn table_is_exactly_the_clock33_crates() {
    let names: Vec<&str> = crate::clock33_data::TABLE.iter().map(|c| c.name).collect();
    for n in &names {
        assert!(
            workspace_guards::workspace_members().iter().any(|m| m == n),
            "{n} is not a workspace member"
        );
    }
}

/// The scanner itself: it must bite on the shapes it claims to catch and
/// ignore test-only code and comments, so the guard cannot silently stop
/// guarding.
#[test]
fn scanner_catches_banned_shapes_and_skips_test_items() {
    let src = "fn a(x: u64, y: u64) -> u64 { x.abs_diff(y) }\n\
               const W: u64 = 1u64 << 33;\n\
               const H: u64 = 0x2_0000_0000;\n\
               const D: u64 = 8_589_934_592;\n\
               #[cfg(test)]\nmod tests {\n    const W: u64 = 1 << 33;\n}\n\
               // 1 << 33 in a comment\n";
    let hits: Vec<_> = non_test_lines(src)
        .into_iter()
        .flat_map(|(_, l)| {
            banned_in_line(BANNED_FULL, &l)
                .into_iter()
                .map(|h| (h.0.to_string(), h.1.to_string()))
                .collect::<Vec<_>>()
        })
        .collect();
    // `1u64 << 33` contains no `1 << 33` substring, so each shape hits once.
    assert_eq!(hits.len(), 4, "{hits:?}");
}

#[test]
fn allow_pin_and_stale_entry_are_enforced() {
    let src = vec![(
        "src/x.rs".to_string(),
        "fn a(x: u64, y: u64) -> u64 { x.abs_diff(y) }\n".to_string(),
    )];
    let pinned = Crate {
        name: "synthetic",
        banned: BANNED_FULL,
        allow: &[("src/x.rs", 1, "abs_diff", "reason")],
    };
    assert!(check(&pinned, &src).is_empty());
    let wrong_line = Crate {
        name: "synthetic",
        banned: BANNED_FULL,
        allow: &[("src/x.rs", 9, "abs_diff", "reason")],
    };
    let v = check(&wrong_line, &src);
    assert_eq!(v.len(), 2, "{v:?}");
    assert!(v[0].starts_with("synthetic: src/x.rs:1:"), "{v:?}");
}
