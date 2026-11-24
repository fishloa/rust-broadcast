//! Magic-number guard (issue #4051), applied to every table crate.
//!
//! Production code may hold no numeric literal other than `0` and `1` outside
//! a constant/static definition. Everything else must be a named constant (or
//! an enum), or a std form (`u8::BITS`, `size_of::<T>()`), so a reader sees the
//! field or role a value belongs to rather than a bare number.
//!
//! Exempt, because the number IS the spec token:
//!
//! - a `const`/`static` definition (and the rest of a multi-line value);
//! - an enum discriminant or a spec-code table row (`CODE = 0x12,`);
//! - a match-arm pattern made only of literals (`0x1 | 0x2 => …`);
//! - a match-arm value that is a bare literal (`"x" => 0x12,`);
//! - test code: `#[cfg(test)]` items, test-only module files, `tests.rs`,
//!   `*_tests.rs`, `testutil`/`test_support` files and anything under
//!   `src/tests/`;
//! - literals inside strings and comments.
//!
//! A constant whose NAME encodes the value (`BIT3_SHIFT`, `BYTE_2_MASK`,
//! `WRAP_33BIT_HALF`) is still flagged: the name must say what the field is,
//! not what the number is.
//!
//! **This is a lexical tripwire, not a proof.** A macro, a split expression or
//! a renamed helper can evade it; code review is the real control. Every
//! `allow` entry is pinned to the exact trimmed source line and states why the
//! hit is not a magic number; a stale entry (no longer flagged) fails.

use workspace_guards::scan;

pub struct Crate {
    pub name: &'static str,
    /// `(path relative to the crate, exact trimmed source line, reason)`.
    pub allow: &'static [(&'static str, &'static str, &'static str)],
}

/// Why a crate has no production wire code to police.
pub type Exemption = (&'static str, &'static str);

/// The files a lexical numeric scan skips by name: a test-only file that holds
/// no test `mod` declaration of its own.
fn is_test_file(rel: &str) -> bool {
    if rel.split('/').any(|seg| seg == "test" || seg == "tests") {
        return true;
    }
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name == "test.rs"
        || name == "tests.rs"
        || name == "testutil.rs"
        || name == "test_support.rs"
        || name
            .strip_suffix(".rs")
            .is_some_and(|stem| stem.ends_with("_test") || stem.ends_with("_tests"))
}

fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// A `0x`/`0b`/`0o` literal body: digits of `is_digit`, at least one.
fn radix_end(b: &[u8], start: usize, is_digit: impl Fn(u8) -> bool) -> Option<usize> {
    let mut j = start;
    while b.get(j).is_some_and(|c| is_digit(*c) || *c == b'_') {
        j += 1;
    }
    (j > start).then_some(j)
}

/// A decimal integer whose leading digit is already consumed: the rest of the
/// digits/underscores and an optional fraction.
fn integer_end(b: &[u8], start: usize) -> usize {
    let mut j = start;
    while b.get(j).is_some_and(|c| c.is_ascii_digit() || *c == b'_') {
        j += 1;
    }
    if b.get(j) == Some(&b'.') && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
        j += 1;
        while b.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
    }
    j
}

/// The end of a fraction (`0.5`, `1.05`) whose `0.`/`1.` prefix is already
/// consumed: the digits and whether any is non-zero.
fn fraction_end(b: &[u8], start: usize) -> Option<usize> {
    let mut j = start;
    while b.get(j).is_some_and(u8::is_ascii_digit) {
        j += 1;
    }
    if j == start || !b[start..j].iter().any(|c| (b'1'..=b'9').contains(c)) {
        return None;
    }
    Some(j)
}

/// The optional `_?[ui](8|16|32|64|128|size)` / `_?f(32|64)` suffix.
fn suffix_end(b: &[u8], start: usize) -> usize {
    let k = if b.get(start) == Some(&b'_') {
        start + 1
    } else {
        start
    };
    let rest = b.get(k..).unwrap_or_default();
    let lengths: &[&[u8]] = match rest.first() {
        Some(b'u') | Some(b'i') => &[b"128", b"size", b"16", b"32", b"64", b"8"],
        Some(b'f') => &[b"32", b"64"],
        _ => return start,
    };
    match lengths.iter().find(|l| rest[1..].starts_with(l)) {
        Some(l) => k + 1 + l.len(),
        None => start,
    }
}

/// The end of the numeric literal starting at `i`, if `b[i..]` begins one:
/// `0x`/`0b`/`0o` forms, a decimal of two or more (`0` and `1` alone are
/// allowed), or a fractional value that is not exactly `0.0`/`1.0`, each with
/// an optional integer/float suffix.
fn literal_end(b: &[u8], i: usize) -> Option<usize> {
    let j = match b[i] {
        b'0' => match b.get(i + 1) {
            Some(b'x') => return radix_end(b, i + 2, |c: u8| c.is_ascii_hexdigit()),
            Some(b'b') => return radix_end(b, i + 2, |c: u8| c == b'0' || c == b'1'),
            Some(b'o') => return radix_end(b, i + 2, |c: u8| (b'0'..=b'7').contains(&c)),
            Some(b'.') => fraction_end(b, i + 2)?,
            _ => return None,
        },
        b'1' => match b.get(i + 1) {
            Some(c) if c.is_ascii_digit() || *c == b'_' => integer_end(b, i + 1),
            Some(b'.') => fraction_end(b, i + 2)?,
            _ => return None,
        },
        b'2'..=b'9' => integer_end(b, i + 1),
        _ => return None,
    };
    Some(suffix_end(b, j))
}

/// The span of the first numeric literal in `line`, if any.
fn literal_span(line: &str) -> Option<(usize, usize)> {
    let b = line.as_bytes();
    for i in 0..b.len() {
        if !b[i].is_ascii_digit() {
            continue;
        }
        let before_ok = i == 0 || !(word_byte(b[i - 1]) || b[i - 1] == b'.' || b[i - 1] == b'\'');
        if !before_ok {
            continue;
        }
        let Some(end) = literal_end(b, i) else {
            continue;
        };
        if b.get(end).is_none_or(|c| !(word_byte(*c) || *c == b'.')) {
            return Some((i, end));
        }
    }
    None
}

fn has_numeric_literal(line: &str) -> bool {
    literal_span(line).is_some()
}

const NUMBER_WORDS: &[&str] = &[
    "ZERO", "ONE", "TWO", "THREE", "FOUR", "FIVE", "SIX", "SEVEN", "EIGHT", "NINE", "TEN",
    "ELEVEN", "TWELVE", "THIRTEEN", "FOURTEEN", "FIFTEEN", "SIXTEEN", "TWENTY", "THIRTY", "FORTY",
    "FIFTY", "SIXTY", "HUNDRED", "THOUSAND", "MILLION", "HALF", "DOUBLE",
];

const POSITION_WORDS: &[&str] = &["SHIFT", "MASK", "OFFSET", "BITS", "LEN", "BYTES", "SIZE"];

const UNIT_WORDS: &[&str] = &["BIT", "BYTE", "WORD"];

/// Whether `word` occurs in `name` delimited by underscore or the name's ends.
fn bounded_word(name: &str, word: &str) -> bool {
    let bytes = name.as_bytes();
    let mut from = 0;
    while let Some(off) = name[from..].find(word) {
        let at = from + off;
        let end = at + word.len();
        let before_ok = at == 0 || bytes[at - 1] == b'_';
        let after_ok = end == bytes.len() || bytes[end] == b'_';
        if before_ok && after_ok {
            return true;
        }
        from = at + 1;
    }
    false
}

/// Whether a constant name encodes its value, so it is not a name of the field
/// or role: a number word (`ZERO`, `HALF`), a `<BIT|BYTE|WORD><n>` unit, or a
/// position word bound to a number (`MASK_3`, `SIZE_192`).
fn value_named(name: &str) -> bool {
    if NUMBER_WORDS.iter().any(|w| bounded_word(name, w)) {
        return true;
    }
    let bytes = name.as_bytes();
    for unit in UNIT_WORDS {
        let mut from = 0;
        while let Some(off) = name[from..].find(unit) {
            let at = from + off;
            let end = at + unit.len();
            if at == 0 || bytes[at - 1] == b'_' {
                let mut j = end;
                if bytes.get(j) == Some(&b'_') {
                    j += 1;
                }
                let digits = j;
                while bytes.get(j).is_some_and(u8::is_ascii_digit) {
                    j += 1;
                }
                if j > digits && (j == bytes.len() || bytes[j] == b'_') {
                    return true;
                }
            }
            from = at + 1;
        }
    }
    let digits_start = name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits_start < name.len() {
        let head = &name[..digits_start];
        for key in POSITION_WORDS {
            let keyword = head.strip_prefix(key);
            if keyword.is_some_and(|after| after.is_empty() || after == "_") {
                return true;
            }
            let suffix = ["_", key, "_"].concat();
            if head.ends_with(&suffix) {
                return true;
            }
        }
    }
    false
}

/// The name of the constant/static a line defines, if it is a definition.
fn const_name(code: &str) -> Option<&str> {
    let mut rest = code;
    if let Some(after) = rest.strip_prefix("pub") {
        let first = after.as_bytes().first().copied();
        if first == Some(b'(') {
            let close = after.find(')')?;
            let inner = &after[1..close];
            if inner.is_empty()
                || !inner
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
            {
                return None;
            }
            let after = &after[close + 1..];
            if !after.starts_with(|c: char| c.is_whitespace()) {
                return None;
            }
            rest = after.trim_start();
        } else if first.is_some_and(|c| c.is_ascii_whitespace()) {
            rest = after.trim_start();
        } else {
            return None;
        }
    }
    let rest = rest
        .strip_prefix("const")
        .or_else(|| rest.strip_prefix("static"))?;
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (!rest[..end].is_empty()).then(|| &rest[..end])
}

/// One literal of a match-arm pattern: `0x…`, `0b…` or decimal digits, with the
/// rest of the line.
fn pattern_literal(s: &str) -> Option<&str> {
    if let Some(r) = s.strip_prefix("0x") {
        let n = r
            .find(|c: char| !(c.is_ascii_hexdigit() || c == '_'))
            .unwrap_or(r.len());
        return (n > 0).then(|| &r[n..]);
    }
    if let Some(r) = s.strip_prefix("0b") {
        let n = r
            .find(|c: char| !(c == '0' || c == '1' || c == '_'))
            .unwrap_or(r.len());
        return (n > 0).then(|| &r[n..]);
    }
    let n = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    (n > 0).then(|| &s[n..])
}

/// A decimal or hex literal (no binary form), as the range bounds allow.
fn range_literal(s: &str) -> Option<&str> {
    if let Some(r) = s.strip_prefix("0x") {
        let n = r
            .find(|c: char| !(c.is_ascii_hexdigit() || c == '_'))
            .unwrap_or(r.len());
        return (n > 0).then(|| &r[n..]);
    }
    let n = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    (n > 0).then(|| &s[n..])
}

/// `^LIT(\s*\|\s*LIT)*\s*(\.\.=?\s*LIT)?\s*=>`: a match-arm pattern made only
/// of literals (a spec-code table's `=>` arms).
fn is_literal_arm(code: &str) -> bool {
    let Some(mut rest) = pattern_literal(code) else {
        return false;
    };
    loop {
        let t = rest.trim_start();
        if let Some(after) = t.strip_prefix('|') {
            match pattern_literal(after.trim_start()) {
                Some(r) => rest = r,
                None => return false,
            }
        } else {
            rest = t;
            break;
        }
    }
    let rest = if let Some(after) = rest.strip_prefix("..=") {
        match range_literal(after.trim_start()) {
            Some(r) => r,
            None => return false,
        }
    } else if let Some(after) = rest.strip_prefix("..") {
        match range_literal(after.trim_start()) {
            Some(r) => r,
            None => return false,
        }
    } else {
        rest
    };
    rest.trim_start().starts_with("=>")
}

/// `^[A-Z]\w*\s*=\s*(0x|0b)?\w+\s*(=>.*)?,?$`: an enum discriminant or a
/// spec-code table row.
fn is_code_row(code: &str) -> bool {
    let Some(eq) = code.find('=') else {
        return false;
    };
    let lhs = code[..eq].trim_end();
    let mut chars = lhs.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_uppercase()) {
        return false;
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '_') {
        return false;
    }
    let rhs = code[eq + 1..].trim_start();
    if rhs.starts_with('[') {
        // A spec-code table row whose value is an array of ranges.
        return true;
    }
    let rhs = rhs
        .strip_prefix("0x")
        .or_else(|| rhs.strip_prefix("0b"))
        .unwrap_or(rhs);
    let n = rhs
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rhs.len());
    if n == 0 {
        return false;
    }
    let rest = rhs[n..].trim();
    rest.is_empty() || rest == "," || rest.starts_with("=>")
}

/// `=>\s*LIT\s*,?$`: a match arm whose value is a bare literal.
fn is_literal_value_arm(code: &str) -> bool {
    let Some(pos) = code.rfind("=>") else {
        return false;
    };
    let Some(rest) = pattern_literal(code[pos + 2..].trim_start()) else {
        return false;
    };
    let rest = rest.trim();
    rest.is_empty() || rest == ","
}

fn check(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut used = vec![false; c.allow.len()];
    for (rel, text) in files {
        if is_test_file(rel) {
            continue;
        }
        let production = scan::without_attributed_items(text, scan::test_starting_attribute);
        let code = scan::blank_comments_and_strings(&production);
        let mut in_const = false;
        for (no, (original, code_line)) in production.lines().zip(code.lines()).enumerate() {
            let trimmed = original.trim();
            if trimmed.starts_with("//")
                || trimmed.starts_with("#[")
                || trimmed.starts_with("#!")
                || trimmed.starts_with('*')
                || trimmed.starts_with("/*")
            {
                continue;
            }
            let code_line = code_line.trim();
            let allow_index = c
                .allow
                .iter()
                .position(|(p, l, _)| *p == rel && *l == trimmed);
            if let Some(name) = const_name(code_line) {
                if value_named(name) {
                    match allow_index {
                        Some(i) => used[i] = true,
                        None => problems.push(format!("{}: {rel}:{}: {trimmed}", c.name, no + 1)),
                    }
                }
                in_const = !code_line.ends_with(';');
                continue;
            }
            if in_const {
                if code_line.ends_with(';') {
                    in_const = false;
                }
                continue;
            }
            if is_code_row(code_line)
                || is_literal_arm(code_line)
                || is_literal_value_arm(code_line)
            {
                continue;
            }
            if !has_numeric_literal(code_line) {
                continue;
            }
            match allow_index {
                Some(i) => used[i] = true,
                None => problems.push(format!("{}: {rel}:{}: {trimmed}", c.name, no + 1)),
            }
        }
    }
    for (i, u) in used.iter().enumerate() {
        if !*u {
            let (path, line, _) = c.allow[i];
            problems.push(format!("{}: stale allowlist entry: {path}: {line}", c.name));
        }
    }
    problems
}

#[test]
fn no_magic_numbers_in_production_code() {
    let mut problems = Vec::new();
    for c in crate::magic_numbers_data::TABLE {
        let files = workspace_guards::src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        problems.extend(check(c, &files));
    }
    assert!(
        problems.is_empty(),
        "numeric literal other than 0 or 1 in production code (name a constant for the field/role, use a std form such as `u8::BITS`/`size_of`, or add a reasoned allowlist entry in magic_numbers_data.rs if it is genuinely not a magic number):\n{}",
        problems.join("\n")
    );
}

#[test]
fn table_covers_every_workspace_member() {
    let names: Vec<&str> = crate::magic_numbers_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    let exempt: Vec<&str> = crate::magic_numbers_data::EXEMPT
        .iter()
        .map(|(name, _)| *name)
        .collect();
    crate::common::assert_table_covers_members("magic_numbers", &names, &exempt);
}

#[test]
fn exempt_entries_are_reasoned() {
    for (name, reason) in crate::magic_numbers_data::EXEMPT {
        assert!(
            !reason.trim().is_empty(),
            "{name}: exemption needs a reason"
        );
    }
}

#[test]
fn allowlist_entries_are_reasoned_and_pinned() {
    for c in crate::magic_numbers_data::TABLE {
        for (path, line, reason) in c.allow {
            assert!(
                !reason.trim().is_empty() && !line.trim().is_empty() && !path.is_empty(),
                "{}: allowlist entry ({path}) needs an exact line pin and a reason",
                c.name
            );
            assert!(
                path.starts_with("src/"),
                "{}: allowlist path is crate-relative: {path}",
                c.name
            );
        }
    }
}

#[test]
fn scanner_flags_a_bare_literal_but_not_zero_or_one() {
    assert!(has_numeric_literal("let x = 42;"));
    assert!(has_numeric_literal("let x = 0x10;"));
    assert!(has_numeric_literal("let x = 1_000u16;"));
    assert!(has_numeric_literal("let x = 0.5;"));
    assert!(!has_numeric_literal("let x = 0;"));
    assert!(!has_numeric_literal("let x = 1;"));
    assert!(!has_numeric_literal("let x = 1u8;"));
    assert!(!has_numeric_literal("let x = 0.0;"));
    assert!(!has_numeric_literal("let x = u8::BITS;"));
}

#[test]
fn scanner_ignores_strings_comments_and_test_items() {
    let c = Crate {
        name: "synthetic",
        allow: &[],
    };
    let files = vec![(
        "src/a.rs".to_string(),
        "fn f() {\n    let s = \"42\";\n    // 42 in a comment\n    let c = '7';\n}\n\
         #[cfg(test)]\nmod tests {\n    const X: u8 = 42;\n}\n"
            .to_string(),
    )];
    assert!(check(&c, &files).is_empty(), "{:?}", check(&c, &files));
}

#[test]
fn scanner_ignores_const_definitions_and_spec_code_rows() {
    let c = Crate {
        name: "synthetic",
        allow: &[],
    };
    let files = vec![(
        "src/a.rs".to_string(),
        "const MASK: u8 = 0x1F;\nconst SPLIT: u16 = 0x1234\n    | 0x5678;\n\
         enum E {\n    A = 3,\n}\n\
         fn f(x: u8) -> &'static str {\n    match x {\n        0x1 | 0x2 => \"a\",\n        0x3..=0x9 => \"b\",\n        _ => \"c\",\n    }\n}\n"
            .to_string(),
    )];
    assert!(check(&c, &files).is_empty(), "{:?}", check(&c, &files));
}

#[test]
fn scanner_flags_a_bare_literal_and_a_value_named_constant() {
    let c = Crate {
        name: "synthetic",
        allow: &[],
    };
    let files = vec![(
        "src/a.rs".to_string(),
        "fn f() -> u8 {\n    let x = 42;\n    x\n}\nconst BYTE_3_MASK: u8 = 7;\n".to_string(),
    )];
    let problems = check(&c, &files);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert_eq!(problems[0], "synthetic: src/a.rs:2: let x = 42;");
    assert_eq!(
        problems[1],
        "synthetic: src/a.rs:5: const BYTE_3_MASK: u8 = 7;"
    );
}

#[test]
fn value_named_constants_are_recognised() {
    for name in [
        "BYTE_3_MASK",
        "BIT3_SHIFT",
        "WORD1_OFFSET",
        "TS_PACKET_SIZE_192",
        "WRAP_33BIT_HALF",
        "SIZE_8",
        "XLINK_RESOLVE_TO_ZERO",
        "ONE_BYTE_PROFILE_ID",
    ] {
        assert!(value_named(name), "{name}");
    }
    for name in [
        "SSI_MASK",
        "CURRENT_NEXT_SHIFT",
        "PTS_DTS_FLAGS_SHIFT",
        "BYTE_MASK",
        "MARKED_FIELD_LEN",
        "FIELD_LEN",
    ] {
        assert!(!value_named(name), "{name}");
    }
}

#[test]
fn an_allowlist_entry_silences_exactly_its_line_and_a_stale_one_fails() {
    let files = vec![(
        "src/a.rs".to_string(),
        "fn f() {\n    let x = 42;\n}\n".to_string(),
    )];
    let pinned = Crate {
        name: "synthetic",
        allow: &[("src/a.rs", "let x = 42;", "synthetic reason")],
    };
    assert!(check(&pinned, &files).is_empty());
    let stale = Crate {
        name: "synthetic",
        allow: &[("src/a.rs", "let x = 43;", "synthetic reason")],
    };
    let problems = check(&stale, &files);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("src/a.rs:2:"), "{problems:?}");
    assert!(
        problems[1].contains("stale allowlist entry"),
        "{problems:?}"
    );
}

#[test]
fn a_test_module_in_the_middle_does_not_hide_the_code_after_it() {
    let files = vec![(
        "src/a.rs".to_string(),
        "#[cfg(test)]\nmod t {\n    fn f() {}\n}\nfn g() -> u8 {\n    let x = 42;\n    x\n}\n"
            .to_string(),
    )];
    let c = Crate {
        name: "synthetic",
        allow: &[],
    };
    let problems = check(&c, &files);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("src/a.rs:6:"), "{problems:?}");
}
