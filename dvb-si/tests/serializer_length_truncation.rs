//! Regression guard for the serializer length/count truncation class (#1129).
//!
//! `descriptors/network_name.rs` used to write `descriptor_length` with
//! `buf[1] = self.network_name.len() as u8;` — no range check, so a body
//! longer than 255 bytes silently wrapped and misframed the rest of the
//! descriptor loop. The same shape (`<expr>.len() as u8/u16/u32`, applied
//! directly to a length/count with no prior range check) recurred across
//! ~40 descriptor serializers.
//!
//! The fix is `broadcast_common::len::fit_u8`/`fit_u16`/`fit_bits`, which
//! compare before narrowing and return `Err` instead of wrapping. This test
//! is the cheap, ungameable half of the regression guard: it fails CI if the
//! literal truncating pattern reappears anywhere under `src/descriptors/`,
//! `src/tables/` or `src/carousel/` outside a `#[cfg(test)]` module (test fixtures are allowed to poke raw
//! bytes together by hand). See also the per-descriptor boundary tests
//! (network_name, telephone, audio_preselection, target_region_name,
//! vvc_subpictures, protection_message, data_broadcast, multilingual_*, …)
//! that prove `serialize_into` actually returns `Err` for an over-range
//! field, which this source scan cannot show by itself.

use std::fs;
use std::path::{Path, PathBuf};

/// Collect `(path, contents)` for every `.rs` under `dir`.
fn read_rs(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            read_rs(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            let body = fs::read_to_string(&path).expect("read .rs");
            out.push((path, body));
        }
    }
}

/// The truncating pattern: a length/count narrowed straight off `.len()`
/// with no intervening range check. Deliberately does NOT match
/// `x.len() as u64` (widening, harmless) or `fit_u8(x.len(), ..)? as u8`
/// (checked first, then narrowed) — only the direct, unchecked narrow.
fn has_truncating_cast(line: &str) -> bool {
    let Some(idx) = line.find(".len()") else {
        return false;
    };
    let rest = line[idx + ".len()".len()..].trim_start();
    for width in ["as u8", "as u16", "as u32"] {
        if let Some(r) = rest.strip_prefix(width) {
            // Must be a cast boundary, not e.g. `as u8x16` or similar.
            if r.chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_')
            {
                return true;
            }
        }
    }
    false
}

/// Byte offset of the first `#[cfg(test)]` in the file, if any. Everything
/// from there on is treated as test fixture code, which is allowed to build
/// raw wire bytes by hand (that's the whole point of a fixture).
fn test_module_start(body: &str) -> Option<usize> {
    body.find("#[cfg(test)]")
}

#[test]
fn no_unchecked_len_narrowing_in_descriptor_serializers() {
    let mut files = Vec::new();
    for dir in ["src/descriptors", "src/tables", "src/carousel"] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
        let before = files.len();
        read_rs(&root, &mut files);
        assert!(files.len() > before, "scan found no files under {dir}");
    }

    let mut problems: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    for (path, body) in &files {
        scanned += 1;
        let production_end = test_module_start(body).unwrap_or(body.len());
        let production = &body[..production_end];

        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(path)
            .display();

        for (i, line) in production.lines().enumerate() {
            if has_truncating_cast(line) {
                problems.push(format!(
                    "{rel}:{}: unchecked `.len() as uN` — use broadcast_common::len::fit_u8/fit_u16/fit_bits instead (#1129): {}",
                    i + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        scanned > 100,
        "scan found only {scanned} files under src/descriptors, src/tables, src/carousel — walk broken?"
    );
    assert!(
        problems.is_empty(),
        "found {} unchecked length-narrowing site(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// The shared section-header masks moved to `tables/mod.rs` by r02-W16: the
/// `reserved(2)='11'`, `reserved(4)='1111'`, `reserved(3)='111'` fills and
/// the 12-bit `section_length` cap used to be bare literals in every table
/// parser and serializer (`0xC0`, `0xF0`, `0xE0`, `0x0F`, `0x0FFF`).
///
/// Named field tags and enum discriminants are legitimate hex literals, so
/// this is a curated per-file allowance rather than a blanket ban — the point
/// is that a *new* mask literal cannot be introduced unnoticed.
/// Table-id byte values, enum discriminants, and static-type tags: legitimate
/// hex literals that are not section-header masks.
const ALLOWED_HEX_OUTSIDE_TESTS: &[(&str, &[&str])] = &[
    (
        "src/tables/pmt.rs",
        &["0x07", "0x0F", "0x10", "0x1F", "0x80"],
    ),
    ("src/tables/ait.rs", &["0x07", "0x80", "0x8000"]),
    ("src/tables/dsmcc.rs", &["0x3A", "0x3F"]),
    ("src/tables/mpe.rs", &["0x3E", "0x3F"]),
    ("src/tables/rct.rs", &["0x30", "0x3F"]),
    ("src/tables/rnt.rs", &["0x80"]),
    ("src/tables/unt.rs", &["0x01", "0x02", "0x80"]),
    ("src/tables/any.rs", &["0x3A", "0x3F"]),
    // 12-bit field maxima (`SECTION_LENGTH_MAX`), not masks: the shared
    // value is named in `tables/mod.rs` but these modules re-derive a local
    // bound for their own error context.
    ("src/tables/sat.rs", &["0x0FFF"]),
    ("src/tables/sit.rs", &["0x0FFF"]),
    ("src/tables/protection_message.rs", &["0x0FFF"]),
    ("src/tables/real_time_parameters.rs", &["0x0FFF"]),
];

/// Whole files whose remaining hex literals are *already* named, documented
/// per-field constants (or a `Reserved(v)` decode mask) for a field with no
/// shared counterpart. Banning the literal there would not add a name — the
/// name is on the line above it — so these are exempt from the mask scan and
/// only from it.
const LOCAL_CONSTANT_ALLOWLIST: &[&str] = &[
    "src/tables/rct.rs",
    "src/tables/downloadable_font_info.rs",
    "src/tables/protection_message.rs",
    "src/tables/sat.rs",
    "src/tables/rnt.rs",
];

/// Hex literals that look like a section-header bit mask.
fn is_mask_literal(token: &str) -> bool {
    matches!(token, "0xC0" | "0xF0" | "0xE0" | "0x0F" | "0x1F" | "0x3F")
}

#[test]
fn no_section_header_mask_literals_outside_tables_mod() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tables");
    let mut files = Vec::new();
    read_rs(&root, &mut files);

    let mut problems: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    for (path, body) in &files {
        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(path)
            .display()
            .to_string();
        if rel.ends_with("tables/mod.rs") {
            continue;
        }
        scanned += 1;
        let production_end = test_module_start(body).unwrap_or(body.len());
        let production = &body[..production_end];
        if LOCAL_CONSTANT_ALLOWLIST.contains(&rel.as_str()) {
            continue;
        }
        let allowed = ALLOWED_HEX_OUTSIDE_TESTS
            .iter()
            .find(|(f, _)| *f == rel)
            .map_or(&[][..], |(_, a)| *a);

        for (i, line) in production.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for token in code.split(|c: char| !(c.is_ascii_alphanumeric() || c == 'x' || c == 'X'))
            {
                if is_mask_literal(token) && !allowed.contains(&token) {
                    problems.push(format!(
                        "{rel}:{}: bare mask literal `{token}` — use the named constant in tables/mod.rs (r02-W16): {}",
                        i + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        scanned > 20,
        "scan found only {scanned} table files — walk broken?"
    );
    assert!(
        problems.is_empty(),
        "found {} bare mask literal(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
