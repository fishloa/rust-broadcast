//! Regression guard for the serializer "silent `& MASK` wrap" class (#1129).
//!
//! A serializer that packs a struct field into a bit-field with
//! `(self.field & MASK) << n` silently drops the high bits of an over-range
//! value (a 4-bit field given `0x13` is written as `3`), emitting a valid-
//! looking but different descriptor. The fix is
//! `broadcast_common::len::fit_mask(value, MASK, "field")?`, which returns
//! `Error::FieldOverflow` instead.
//!
//! This is the cheap lexical half of the guard (the behavioural half is
//! `serialize_field_width_guard.rs`): it fails CI if a single-field `& MASK`
//! reappears in a serialize-side function under `src/` outside a
//! `#[cfg(test)]` module. A tripwire is lexical, not a proof; code review is
//! the real control.
//!
//! What counts as a hit: a ` & ` whose right operand is a mask (a hex literal
//! other than the byte-extraction `0xFF`, or an ALL_CAPS constant) and whose
//! left operand is a struct field / enum value (`self.x`, `e.x`, `x.to_u8()`),
//! in a function that is not on the parse/accessor side, and which is not a
//! pure boolean flag test (`(x & MASK) != 0`). Splitting an already
//! range-checked *local* across two bytes (`(lt & 0x0F) << 4`,
//! `(v >> 8) as u8 & 0x0F`) is not a hit: the whole field is checked up front
//! and the mask is only a bit-split.

use std::fs;
use std::path::{Path, PathBuf};

/// `(file suffix, line substring, reason)` — each exemption is pinned to the
/// exact source text so a moved/changed line re-opens the question.
const ALLOW: &[(&str, &str, &str)] = &[
    (
        "tables/rct.rs",
        "let sd_lo = (loc.start_date & 0x3F) as u8;",
        "start_date is range-checked to 9 bits (fit_mask) at the top of serialize_locator; this only splits the low 6 bits into byte 1",
    ),
    (
        "descriptors/extension/dts_hd.rs",
        "(a.bit_rate_or_scaled as u8 & 0x3F) << 2",
        "bit_rate_or_scaled is rejected above MAX_BIT_RATE (14 bits) earlier in serialize_into; this splits its low 6 bits",
    ),
    (
        "descriptors/dts.rs",
        "u64::from(self.",
        "every packed field is rejected above its *_MAX with a typed InvalidDescriptor at the top of serialize_into",
    ),
    (
        "descriptors/cable_delivery_system.rs",
        "(self.symbol_rate_bcd & 0x0F) as u8) << 4",
        "symbol_rate_bcd is the full 32-bit u32 (28-bit value + low nibble); nothing can overflow, this extracts the low nibble",
    ),
    (
        "descriptors/video_window.rs",
        "(self.vertical_offset & 0x0F) << 4",
        "vertical_offset is range-checked to 14 bits (fit_bits) earlier in serialize_into; this splits its low 4 bits",
    ),
    (
        "descriptors/target_background_grid.rs",
        "(self.vertical_size & 0x0F) << 4",
        "vertical_size is range-checked to 14 bits (fit_bits) earlier in serialize_into; this splits its low 4 bits",
    ),
];

fn read_rs(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            read_rs(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push((path.clone(), fs::read_to_string(&path).expect("read .rs")));
        }
    }
}

/// Function-name prefixes that are on the parse / accessor side: a mask there
/// extracts bits from received bytes and cannot overflow a field.
const READ_SIDE: &[&str] = &[
    "parse",
    "read",
    "decode",
    "from",
    "try_from",
    "is_",
    "has_",
    "name",
    "to_",
    "flags",
    "next",
    "iter",
    "get",
    "find",
    "check",
    "lookup",
    "contains",
    "matches",
    "section",
    "header",
    "pil_",
    "receiver_",
    "subcell_",
    "cell_",
    "version",
    "consider",
    "b1",
    "bits",
    "sext",
    "bit_rate",
];

fn is_mask_token(tok: &str) -> bool {
    if let Some(hex) = tok.strip_prefix("0x") {
        let digits: String = hex.chars().filter(|c| *c != '_').collect();
        return !digits.is_empty() && !digits.eq_ignore_ascii_case("ff");
    }
    tok.len() > 2
        && tok
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && tok.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

/// Is the operand immediately left of the `&` a struct field / enum value
/// (as opposed to a range-checked local being bit-split)?
fn lhs_is_field(lhs: &str) -> bool {
    // Take the operand: back to the previous top-level `(`, `|`, `=`, `,`.
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in lhs.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    start = i + 1;
                    break;
                }
                depth -= 1;
            }
            '|' | '=' | ',' if depth == 0 => {
                start = i + 1;
                break;
            }
            _ => {}
        }
    }
    let operand = lhs[start..].trim();
    if operand.contains(">>") || operand.contains("<<") {
        return false;
    }
    operand.contains("self.")
        || operand.contains(".to_u8()")
        || operand.contains(".to_u16()")
        || operand.contains(".unwrap_or(")
        || operand.contains(".ok_or(")
        || (operand.contains('.') && !operand.contains(".len()"))
}

/// `(self.field << N)` / `(e.field.to_u8() << N)`: a bare struct field or enum
/// code shifted into a packed byte with no range check (a `u8` shift silently
/// drops the high bits).
fn shifts_bare_field(ctx: &str) -> bool {
    let b = ctx.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'<' && b[i + 1] == b'<' && (i == 0 || b[i - 1] != b'<') {
            let lhs = ctx[..i].trim_end();
            let start = lhs
                .rfind(|c: char| {
                    !(c.is_alphanumeric() || c == '_' || c == '.' || c == '(' || c == ')')
                })
                .map_or(0, |p| p + 1);
            let operand = lhs[start..].trim_start_matches('(');
            let operand = operand.strip_suffix(".to_u8()").unwrap_or(operand);
            if operand.starts_with("self.")
                && operand[5..]
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_')
            {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn hits_in(body: &str) -> Vec<(usize, String)> {
    let end = body.find("#[cfg(test)]").unwrap_or(body.len());
    let mut cur_fn = String::new();
    let mut out = Vec::new();
    // Join each statement across lines so a multi-line `x\n & MASK` is seen.
    let lines: Vec<&str> = body[..end].lines().collect();
    for (n, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if let Some(rest) = line.split("fn ").nth(1).filter(|_| {
            line.starts_with("fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("pub(crate) fn ")
                || line.starts_with("pub const fn ")
                || line.starts_with("const fn ")
        }) {
            cur_fn = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
        }
        if line.starts_with("//") || line.contains("fit_mask") {
            continue;
        }
        if READ_SIDE.iter().any(|p| cur_fn.starts_with(p)) {
            continue;
        }
        // Look at the line plus the previous one (rustfmt breaks `x\n & M`).
        let mut ctx = String::new();
        if n > 0 && line.starts_with('&') {
            ctx.push_str(lines[n - 1].trim());
            ctx.push(' ');
        }
        ctx.push_str(line);
        let bytes: Vec<char> = ctx.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == '&'
                && bytes.get(i + 1) != Some(&'&')
                && (i == 0 || bytes[i - 1] != '&')
                && (i == 0 || bytes[i - 1] != '(')
                && bytes.get(i + 1) == Some(&' ')
            {
                let rhs: String = bytes[i + 2..]
                    .iter()
                    .take_while(|c| c.is_alphanumeric() || **c == '_')
                    .collect();
                let after: String = bytes[i + 2 + rhs.len()..].iter().collect();
                let lhs: String = bytes[..i].iter().collect();
                let is_flag_test = {
                    let a = after.trim_start_matches(')').trim_start();
                    a.starts_with("!= 0") || a.starts_with("== 0") || a.starts_with("!=")
                };
                if is_mask_token(&rhs) && !is_flag_test && lhs_is_field(&lhs) {
                    out.push((n + 1, line.to_string()));
                    break;
                }
            }
            i += 1;
        }
        if shifts_bare_field(&ctx) && !out.iter().any(|(l, _)| *l == n + 1) {
            out.push((n + 1, line.to_string()));
        }
    }
    out
}

#[test]
fn no_unchecked_field_mask_in_serializers() {
    let mut files = Vec::new();
    read_rs(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 100, "scan found too few files under src");
    let mut problems = Vec::new();
    for (path, body) in &files {
        for (n, line) in hits_in(body) {
            let p = path.to_string_lossy();
            if ALLOW
                .iter()
                .any(|(f, sub, _)| p.ends_with(f) && line.contains(sub))
            {
                continue;
            }
            problems.push(format!("{p}:{n}: {line}"));
        }
    }
    assert!(
        problems.is_empty(),
        "single-field `& MASK` in a serializer silently wraps an over-range \
         value; use broadcast_common::len::fit_mask(value, MASK, \"field\")? \
         (or allow-list with a reason):\n{}",
        problems.join("\n")
    );
}

/// The scanner must bite: a synthetic serializer containing the bad pattern
/// is flagged, the checked form and a parse-side extraction are not.
#[test]
fn scanner_bites_on_the_bad_pattern() {
    let bad = "fn serialize_into(&self) {\n    let b = (self.reserved & RESERVED_MASK) << 4;\n}\n";
    assert_eq!(hits_in(bad).len(), 1, "bad pattern must be flagged");
    let bad_to_u8 = "fn serialize_into(&self) {\n    let b = self.kind.to_u8() & 0x0F;\n}\n";
    assert_eq!(hits_in(bad_to_u8).len(), 1);
    let ok = "fn serialize_into(&self) {\n    let b = fit_mask(self.reserved, RESERVED_MASK, \"reserved\")? << 4;\n}\n";
    assert!(hits_in(ok).is_empty());
    let parse = "fn parse(b: &[u8]) {\n    let r = (self.x & RESERVED_MASK) >> 4;\n}\n";
    assert!(hits_in(parse).is_empty());
    let split = "fn serialize_into(&self) {\n    let b = (lt & 0x0F) << 4;\n}\n";
    assert!(hits_in(split).is_empty());
    let bare_shift = "fn serialize_into(&self) {\n    buf[0] = (self.version << 4) | 1;\n}\n";
    assert_eq!(
        hits_in(bare_shift).len(),
        1,
        "bare field shift must be flagged"
    );
    let checked_shift = "fn serialize_into(&self) {\n    buf[0] = (fit_mask(self.version, 0x0F, \"v\")? << 4) | 1;\n}\n";
    assert!(hits_in(checked_shift).is_empty());
}
