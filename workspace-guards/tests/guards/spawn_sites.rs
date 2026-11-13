//! Tripwire guard (issue #2533, B6.2): every task a runtime crate's tokio
//! adapter detaches has a named owner and a cancel path.
//!
//! The sans-IO runtimes spawn nothing; their socket adapters spawn a few
//! tasks, each owned by a handle whose drop aborts it. A new spawn site in a
//! crate in [`crate::spawn_sites_data::TABLE`] fails here until it is added
//! with its owner and cancel path, so a detached task cannot appear
//! unreviewed. Scans every `src/**/*.rs` outside `#[cfg(test)]` items for the
//! spawn needles below.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper or a spawn
//! through a stored `Handle` evades it; code review is the real control. Every
//! entry is keyed by file, enclosing `fn` and the spawn's ordinal within that fn,
//! so a refactor that only moves lines never breaks it; a stale entry (no longer
//! matching) fails.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// `(path suffix, enclosing fn name (`name@2` for the second declaration of that name in the
    /// file, `name@3` for the third), 1-based ordinal of the spawn among the
    /// spawns inside that fn in that file, owner and cancel path)`.
    pub sites: &'static [(&'static str, &'static str, usize, &'static str)],
}

const NEEDLES: &[&str] = &[
    "tokio::spawn(",
    "task::spawn(",
    ".spawn(",
    "spawn_blocking(",
    "spawn_local(",
    "thread::spawn(",
];

/// `line` without a trailing `//` comment (a `//` inside a string literal stays).
fn code_part(line: &str) -> &str {
    let mut in_string = false;
    let mut previous = '\0';
    for (at, c) in line.char_indices() {
        match c {
            '"' if previous != '\\' => in_string = !in_string,
            '/' if !in_string && previous == '/' => return &line[..at - 1],
            _ => {}
        }
        previous = c;
    }
    line
}

/// The name a line declares with `fn`, if any.
fn declared_fn(text: &str) -> Option<&str> {
    let mut rest = text;
    while let Some(at) = rest.find("fn ") {
        let before_ok = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let after = &rest[at + 3..];
        let name_len = after
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        if before_ok && name_len > 0 {
            return Some(&after[..name_len]);
        }
        rest = after;
    }
    None
}

fn check(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut used = vec![false; c.sites.len()];
    for (rel, text) in files {
        let mut current = "<top>".to_string();
        let mut ordinal = 0usize;
        let mut declarations: std::collections::HashMap<String, usize> = Default::default();
        for (no, line) in workspace_guards::scan::non_test_source_blanked(text)
            .lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l))
            .filter(|(_, l)| !l.trim_start().starts_with("//"))
        {
            if let Some(name) = declared_fn(code_part(line)) {
                let seen = declarations.entry(name.to_string()).or_insert(0);
                *seen += 1;
                current = if *seen == 1 {
                    name.to_string()
                } else {
                    format!("{name}@{seen}")
                };
                ordinal = 0;
            }
            if !NEEDLES.iter().any(|n| line.contains(n)) {
                continue;
            }
            ordinal += 1;
            match c
                .sites
                .iter()
                .position(|(s, f, o, _)| rel.ends_with(s) && *f == current && *o == ordinal)
            {
                Some(i) => used[i] = true,
                None => violations.push(format!(
                    "{}: {rel}:{no} (fn {current}, spawn #{ordinal}): untracked spawn site (add an entry with owner + cancel path): {}",
                    c.name,
                    line.trim()
                )),
            }
        }
    }
    for (site, used) in c.sites.iter().zip(&used) {
        if !*used {
            violations.push(format!(
                "{}: stale site entry: ({}, fn {}, spawn #{})",
                c.name, site.0, site.1, site.2
            ));
        }
    }
    violations
}

#[test]
fn every_spawn_site_in_a_runtime_adapter_is_pinned() {
    let mut violations = Vec::new();
    for c in crate::spawn_sites_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no source files under src/", c.name);
        violations.extend(check(c, &files));
    }
    assert!(
        violations.is_empty(),
        "spawn-site guard violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn table_names_are_workspace_members() {
    let names: Vec<&str> = crate::spawn_sites_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    crate::common::assert_names_are_members("spawn_sites", &names);
}

/// The guard bites: a spawn outside the table is reported, an entry that no
/// longer matches is reported as stale, and moving the spawn to other lines
/// changes nothing.
#[test]
fn the_guard_reports_an_untracked_and_a_stale_site_and_ignores_line_moves() {
    const SOURCE: &str = "fn f() {\n    tokio::spawn(async {});\n}\n";
    let files = vec![("src/lib.rs".to_string(), SOURCE.to_string())];
    let untracked = Crate {
        name: "t",
        sites: &[],
    };
    assert_eq!(check(&untracked, &files).len(), 1);
    let stale = Crate {
        name: "t",
        sites: &[
            ("src/lib.rs", "f", 1, "owner"),
            ("src/gone.rs", "g", 1, "gone"),
        ],
    };
    let reported = check(&stale, &files);
    assert_eq!(reported.len(), 1);
    assert!(reported[0].contains("stale"));

    let moved = vec![(
        "src/lib.rs".to_string(),
        format!("\n\n// moved\n\n{SOURCE}"),
    )];
    let listed = Crate {
        name: "t",
        sites: &[("src/lib.rs", "f", 1, "owner")],
    };
    assert!(check(&listed, &moved).is_empty());
    let second = vec![(
        "src/lib.rs".to_string(),
        "fn f() {\n    tokio::spawn(async {});\n    tokio::spawn(async {});\n}\n".to_string(),
    )];
    assert_eq!(check(&listed, &second).len(), 1);
}

/// two same-named fns in one file share one key, so a single entry covers a spawn in each.
#[test]
fn a_second_spawn_in_a_same_named_fn_is_not_hidden_by_the_first_entry() {
    const SOURCE: &str = "impl A {\n    fn run() {\n        tokio::spawn(async {});\n    }\n}\nimpl B {\n    fn run() {\n        tokio::spawn(async {});\n    }\n}\n";
    let files = vec![("src/lib.rs".to_string(), SOURCE.to_string())];
    let one_entry = Crate {
        name: "t",
        sites: &[("src/lib.rs", "run", 1, "owner of A::run")],
    };
    let reported = check(&one_entry, &files);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(reported[0].contains("run@2"), "{reported:?}");
    let both = Crate {
        name: "t",
        sites: &[
            ("src/lib.rs", "run", 1, "owner of A::run"),
            ("src/lib.rs", "run@2", 1, "owner of B::run"),
        ],
    };
    assert!(check(&both, &files).is_empty());
}

/// `tokio::task::spawn(` contains none of the needles.
#[test]
fn tokio_task_spawn_is_a_spawn_site() {
    const SOURCE: &str = "fn f() {\n    tokio::task::spawn(async {});\n}\n";
    let files = vec![("src/lib.rs".to_string(), SOURCE.to_string())];
    let none = Crate {
        name: "t",
        sites: &[],
    };
    assert_eq!(
        check(&none, &files).len(),
        1,
        "tokio::task::spawn must be reported"
    );
}

/// a trailing comment mentioning `fn x` re-labels the enclosing fn.
#[test]
fn a_comment_naming_a_fn_does_not_change_the_enclosing_fn() {
    const SOURCE: &str =
        "fn real() {\n    let x = 1; // see fn decoy\n    tokio::spawn(async {});\n}\n";
    let files = vec![("src/lib.rs".to_string(), SOURCE.to_string())];
    let c = Crate {
        name: "t",
        sites: &[("src/lib.rs", "real", 1, "owner")],
    };
    assert!(
        check(&c, &files).is_empty(),
        "the spawn is in `real`, entry says real"
    );
}
