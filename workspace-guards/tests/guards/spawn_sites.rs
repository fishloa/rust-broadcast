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
//! entry is line-pinned; a stale entry (no longer matching) fails.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// `(path suffix, line number, owner and cancel path)`.
    pub sites: &'static [(&'static str, usize, &'static str)],
}

const NEEDLES: &[&str] = &[
    "tokio::spawn(",
    ".spawn(",
    "spawn_blocking(",
    "spawn_local(",
    "thread::spawn(",
];

fn check(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut used = vec![false; c.sites.len()];
    for (rel, text) in files {
        for (no, line) in workspace_guards::scan::non_test_source_blanked(text)
            .lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l))
            .filter(|(_, l)| !l.trim_start().starts_with("//"))
        {
            if !NEEDLES.iter().any(|n| line.contains(n)) {
                continue;
            }
            match c
                .sites
                .iter()
                .position(|(s, l, _)| rel.ends_with(s) && *l == no)
            {
                Some(i) => used[i] = true,
                None => violations.push(format!(
                    "{}: {rel}:{no}: untracked spawn site (add an entry with owner + cancel path): {}",
                    c.name,
                    line.trim()
                )),
            }
        }
    }
    for (site, used) in c.sites.iter().zip(&used) {
        if !*used {
            violations.push(format!(
                "{}: stale site entry: ({}, {})",
                c.name, site.0, site.1
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

/// The guard bites: a spawn outside the table is reported, and an entry that no
/// longer matches is reported as stale.
#[test]
fn the_guard_reports_an_untracked_and_a_stale_site() {
    const SOURCE: &str = "fn f() {\n    tokio::spawn(async {});\n}\n";
    let files = vec![("src/lib.rs".to_string(), SOURCE.to_string())];
    let untracked = Crate {
        name: "t",
        sites: &[],
    };
    assert_eq!(check(&untracked, &files).len(), 1);
    let stale = Crate {
        name: "t",
        sites: &[("src/lib.rs", 2, "owner"), ("src/gone.rs", 9, "gone")],
    };
    let reported = check(&stale, &files);
    assert_eq!(reported.len(), 1);
    assert!(reported[0].contains("stale"));
}
