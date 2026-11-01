//! Serializer length/count truncation guard (#1129), applied to every table crate.
//!
//! A wire length/count must be range-checked before it is narrowed:
//! `buf[0] = x.len() as u8;` silently wraps a 256-byte body to length 0 and
//! misframes everything after it. The fix is
//! `broadcast_common::len::fit_u8`/`fit_u16`/`fit_u32`/`fit_bits` (compare in
//! `usize`, then narrow) or an explicit `> MAX` early return.
//!
//! This is the cheap lexical half of the guard (the behavioural half is each
//! type's own over-range test): no production line (outside `#[cfg(test)]`)
//! may narrow `<expr>.len()` / `<expr>.count()` with `as u8|u16|u32` unless
//! its enclosing function also contains a range check, or the exact line is on
//! the crate's `allowlist` with a reason. A stale allowlist entry fails the
//! guard so the list cannot rot.
//!
//! Not in the table, by design: `dvb-si` (its guard scans only
//! `descriptors/`, `tables/` and `carousel/` with a different detector) and
//! `transmux` (its file also carries behavioural box-range tests that need the
//! crate's types).

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// `(file, exact trimmed line, reason)`.
    pub allowlist: &'static [(&'static str, &'static str, &'static str)],
}

/// `.len()` / `.count()` immediately narrowed with `as u8|u16|u32`.
fn narrows_len(line: &str) -> bool {
    let mut rest = line;
    while let Some(i) = rest.find(')') {
        let (head, tail) = rest.split_at(i + 1);
        rest = tail;
        if head.ends_with(".len()") || head.ends_with(".count()") {
            let t = tail.trim_start();
            for w in ["as u8", "as u16", "as u32"] {
                if t.strip_prefix(w).is_some_and(|r| {
                    r.chars()
                        .next()
                        .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                }) {
                    return true;
                }
            }
        }
    }
    false
}

/// Does the function text contain a range check?
fn has_guard(text: &str) -> bool {
    ["fit_", "FieldOverflow", "try_from", "::MAX", "_MAX", "MAX_"]
        .iter()
        .any(|t| text.contains(t))
        || text.contains("> 0x")
        || text.lines().any(|l| {
            let l = l.trim_start();
            (l.starts_with("if ") || l.contains("&& ")) && (l.contains(" > ") || l.contains(" >= "))
        })
}

fn check(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut used = vec![false; c.allowlist.len()];
    for (rel, body) in files {
        let prod = &body[..body.find("#[cfg(test)]").unwrap_or(body.len())];
        let lines: Vec<&str> = prod.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let t = line.trim();
            if t.starts_with("//") || !narrows_len(t) {
                continue;
            }
            if let Some(k) = c
                .allowlist
                .iter()
                .position(|(f, l, _)| *f == rel && *l == t)
            {
                used[k] = true;
                continue;
            }
            let mut start = n;
            while start > 0 && !lines[start].contains("fn ") {
                start -= 1;
            }
            if !has_guard(&lines[start..=n].join("\n")) {
                problems.push(format!(
                    "{}: {rel}:{}: unchecked length narrowing (use broadcast_common::len::fit_*): {t}",
                    c.name,
                    n + 1
                ));
            }
        }
    }
    for (k, u) in used.iter().enumerate() {
        if !*u {
            problems.push(format!(
                "{}: stale allowlist entry: {} :: {}",
                c.name, c.allowlist[k].0, c.allowlist[k].1
            ));
        }
    }
    problems
}

#[test]
fn len_narrowing_sites_are_range_checked_or_allowlisted() {
    let mut problems = Vec::new();
    for c in crate::truncation_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: scan found no source files", c.name);
        problems.extend(check(c, &files));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn table_covers_every_workspace_member() {
    let names: Vec<&str> = crate::truncation_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    crate::common::assert_table_covers_members(
        "truncation",
        &names,
        crate::truncation_data::EXEMPT,
    );
}

#[test]
fn allowlist_entries_are_reasoned() {
    for c in crate::truncation_data::TABLE {
        for (file, line, reason) in c.allowlist {
            assert!(
                !reason.trim().is_empty() && !line.is_empty(),
                "{}: allowlist entry ({file}) needs a line pin and a reason",
                c.name
            );
        }
    }
}

#[test]
fn check_flags_a_synthetic_offender_and_a_stale_entry() {
    let c = Crate {
        name: "synthetic",
        allowlist: &[("src/gone.rs", "x.len() as u8;", "stale on purpose")],
    };
    let files = vec![(
        "src/a.rs".to_string(),
        "fn f() { buf[0] = x.len() as u8; }\n".to_string(),
    )];
    let p = check(&c, &files);
    assert_eq!(p.len(), 2, "{p:?}");
    assert!(p[0].starts_with("synthetic: src/a.rs:1:"), "{p:?}");
    assert!(p[1].contains("stale allowlist entry"), "{p:?}");
}

#[test]
fn guard_detector_bites() {
    assert!(narrows_len("buf[0] = x.len() as u8;"));
    assert!(narrows_len("f(&(a.b.count() as u32));"));
    assert!(!narrows_len("let n = x.len() as u64;"));
    assert!(!narrows_len("let n = x.len() as usize;"));
    assert!(!has_guard("fn f() { buf[0] = x.len() as u8; }"));
    assert!(has_guard("fn f() { let n = fit_u8(x.len(), \"a\")?; }"));
    assert!(has_guard("fn f() {\n if x.len() > 255 { return; }\n }"));
}
