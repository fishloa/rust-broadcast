//! Tripwire guard (umbrella #1137 T9): 33-bit PTS/DTS wrap arithmetic and the
//! 5-byte marker-bit timestamp codec in this crate go through
//! `broadcast_common::clock33` (`WRAP_33BIT`, `unpack_marked`, `pack_marked`),
//! never a hand-rolled modulus, shift-by-33 or bit-packing. Scans every
//! `src/**/*.rs` outside `#[cfg(test)]` items and fails on:
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

use std::fs;
use std::path::{Path, PathBuf};

/// `(needle, why it is banned)`.
const BANNED: &[(&str, &str)] = &[
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
];

/// `(path suffix, line number, needle, reason)` — reviewed, line-pinned
/// exceptions.
const ALLOW: &[(&str, usize, &str, &str)] = &[];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

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

fn banned_in_line(line: &str) -> Vec<(&'static str, &'static str)> {
    BANNED
        .iter()
        .filter(|(needle, _)| line.contains(needle))
        .copied()
        .collect()
}

#[test]
fn no_hand_rolled_33bit_wrap_arithmetic_in_src() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src_dir, &mut files);
    assert!(!files.is_empty());
    let mut violations = Vec::new();
    let mut used = vec![false; ALLOW.len()];
    for file in files {
        let text = fs::read_to_string(&file).expect("read source");
        let rel = file
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for (no, line) in non_test_lines(&text) {
            for (needle, why) in banned_in_line(&line) {
                if let Some(i) = ALLOW
                    .iter()
                    .position(|(s, l, n, _)| rel.ends_with(s) && *l == no && *n == needle)
                {
                    used[i] = true;
                } else {
                    violations.push(format!("{rel}:{no}: {why}: {}", line.trim()));
                }
            }
        }
    }
    let stale: Vec<_> = ALLOW
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(a, _)| (a.0, a.1))
        .collect();
    assert!(
        violations.is_empty(),
        "hand-rolled 33-bit wrap arithmetic (use broadcast_common::clock33, or add a reasoned line-pinned ALLOW entry):\n{}",
        violations.join("\n")
    );
    assert!(stale.is_empty(), "stale ALLOW entries: {stale:?}");
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
        .flat_map(|(_, l)| banned_in_line(&l))
        .collect();
    // `1u64 << 33` contains no `1 << 33` substring, so each shape hits once.
    assert_eq!(hits.len(), 4, "{hits:?}");
}
