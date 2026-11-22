//! Dependency policy: one external crate per purpose across the suite, and
//! none from the banned list (never chrono next to jiff). Reads every declared
//! dependency of every package in every workspace through
//! `cargo metadata --no-deps --offline`, by the real package name (a rename
//! does not hide a crate). Path dependencies and dependencies on root
//! workspace members are not external.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;
use workspace_guards::workspace_root;

use crate::dep_policy_data::{BANNED, EXEMPT, PURPOSES, WORKSPACES, WORKSPACES_EXEMPT};

pub struct Purpose {
    pub name: &'static str,
    pub single: bool,
    pub crates: &'static [&'static str],
}

pub struct Banned {
    pub krate: &'static str,
    pub instead: &'static str,
}

pub struct Exempt {
    pub member: &'static str,
    pub krate: &'static str,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Dep {
    member: String,
    krate: String,
    kind: String,
}

struct Workspace {
    packages: Vec<String>,
    manifests: Vec<PathBuf>,
    meta: Value,
}

fn metadata(root: &str) -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let manifest = workspace_root().join(root).join("Cargo.toml");
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .unwrap_or_else(|e| panic!("run cargo metadata for {}: {e}", manifest.display()));
    assert!(
        out.status.success(),
        "cargo metadata for {} failed: {}",
        manifest.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("parse cargo metadata for {}: {e}", manifest.display()))
}

fn packages(meta: &Value) -> &[Value] {
    meta["packages"].as_array().map_or(&[], Vec::as_slice)
}

fn workspaces() -> &'static BTreeMap<&'static str, Workspace> {
    static CELL: OnceLock<BTreeMap<&'static str, Workspace>> = OnceLock::new();
    CELL.get_or_init(|| {
        WORKSPACES
            .iter()
            .map(|root| {
                let meta = metadata(root);
                let pkgs = packages(&meta);
                let ws = Workspace {
                    packages: pkgs
                        .iter()
                        .filter_map(|p| p["name"].as_str().map(str::to_string))
                        .collect(),
                    manifests: pkgs
                        .iter()
                        .filter_map(|p| p["manifest_path"].as_str().map(PathBuf::from))
                        .collect(),
                    meta,
                };
                (*root, ws)
            })
            .collect()
    })
}

fn root_members() -> BTreeSet<String> {
    workspaces()["."].packages.iter().cloned().collect()
}

fn external_deps(meta: &Value, internal: &BTreeSet<String>) -> Vec<Dep> {
    let mut deps = Vec::new();
    for p in packages(meta) {
        let member = p["name"].as_str().unwrap_or_default();
        for d in p["dependencies"].as_array().map_or(&[][..], Vec::as_slice) {
            let krate = d["name"].as_str().unwrap_or_default();
            if d.get("path").is_some_and(|v| !v.is_null()) || internal.contains(krate) {
                continue;
            }
            deps.push(Dep {
                member: member.to_string(),
                krate: krate.to_string(),
                kind: d["kind"].as_str().unwrap_or("normal").to_string(),
            });
        }
    }
    deps.sort();
    deps.dedup();
    deps
}

fn all_deps() -> Vec<Dep> {
    let internal = root_members();
    let mut deps: Vec<Dep> = workspaces()
        .values()
        .flat_map(|ws| external_deps(&ws.meta, &internal))
        .collect();
    deps.sort();
    deps.dedup();
    deps
}

fn is_exempt(dep: &Dep, exempt: &[Exempt]) -> bool {
    exempt
        .iter()
        .any(|e| e.member == dep.member && e.krate == dep.krate)
}

fn matches(pattern: &str, krate: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => krate.starts_with(prefix),
        None => pattern == krate,
    }
}

fn banned_problems(deps: &[Dep], banned: &[Banned], exempt: &[Exempt]) -> Vec<String> {
    deps.iter()
        .filter(|d| !is_exempt(d, exempt))
        .filter_map(|d| {
            banned.iter().find(|b| matches(b.krate, &d.krate)).map(|b| {
                format!(
                    "dep_policy: `{}` ({}) depends on banned `{}`; use {} instead",
                    d.member, d.kind, d.krate, b.instead
                )
            })
        })
        .collect()
}

fn users(deps: &[Dep], krate: &str, exempt: &[Exempt]) -> Vec<String> {
    let set: BTreeSet<String> = deps
        .iter()
        .filter(|d| d.krate == krate && !is_exempt(d, exempt))
        .map(|d| format!("{} ({})", d.member, d.kind))
        .collect();
    set.into_iter().collect()
}

fn purpose_problems(deps: &[Dep], purposes: &[Purpose], exempt: &[Exempt]) -> Vec<String> {
    let crates: BTreeSet<&str> = deps
        .iter()
        .filter(|d| !is_exempt(d, exempt))
        .map(|d| d.krate.as_str())
        .collect();
    let mut problems = Vec::new();
    for krate in crates {
        let owners: Vec<&str> = purposes
            .iter()
            .filter(|p| p.crates.contains(&krate))
            .map(|p| p.name)
            .collect();
        let members = users(deps, krate, exempt).join(", ");
        match owners.len() {
            1 => {}
            0 => problems.push(format!(
                "dep_policy: `{krate}` (used by {members}) is in no purpose; add it to one purpose in dep_policy_data.rs with reviewer sign-off, or use the crate that already serves that purpose"
            )),
            _ => problems.push(format!(
                "dep_policy: `{krate}` (used by {members}) is listed under several purposes {owners:?}; it must serve exactly one"
            )),
        }
    }
    problems
}

fn single_problems(deps: &[Dep], purposes: &[Purpose], exempt: &[Exempt]) -> Vec<String> {
    let used: BTreeSet<&str> = deps
        .iter()
        .filter(|d| !is_exempt(d, exempt))
        .map(|d| d.krate.as_str())
        .collect();
    let mut problems = Vec::new();
    for p in purposes {
        if p.single && p.crates.len() != 1 {
            problems.push(format!(
                "dep_policy: single purpose `{}` lists {:?}; one crate per purpose",
                p.name, p.crates
            ));
        }
        let present: Vec<&str> = p
            .crates
            .iter()
            .copied()
            .filter(|c| used.contains(c))
            .collect();
        if p.single && present.len() > 1 {
            problems.push(format!(
                "dep_policy: purpose `{}` is served by {present:?}; one crate per purpose",
                p.name
            ));
        }
    }
    problems
}

#[test]
fn no_banned_direct_dependency() {
    let problems = banned_problems(&all_deps(), BANNED, EXEMPT);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_direct_dependency_has_a_purpose() {
    let problems = purpose_problems(&all_deps(), PURPOSES, EXEMPT);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn one_crate_per_purpose() {
    let problems = single_problems(&all_deps(), PURPOSES, EXEMPT);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn no_banned_crate_is_a_purpose() {
    let problems: Vec<String> = PURPOSES
        .iter()
        .flat_map(|p| p.crates.iter().map(move |c| (p.name, *c)))
        .filter(|(_, c)| BANNED.iter().any(|b| matches(b.krate, c)))
        .map(|(p, c)| format!("dep_policy: `{c}` is both banned and the crate for `{p}`"))
        .collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn exemptions_are_live() {
    let deps = all_deps();
    let problems: Vec<String> = EXEMPT
        .iter()
        .filter(|e| {
            !deps
                .iter()
                .any(|d| d.member == e.member && d.krate == e.krate)
        })
        .map(|e| {
            format!(
                "dep_policy: exemption `{}` -> `{}` ({}) matches no dependency; delete it",
                e.member, e.krate, e.reason
            )
        })
        .collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn tracked_manifests(root: &Path) -> Vec<PathBuf> {
    let out = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "Cargo.toml",
            "*/Cargo.toml",
        ])
        .current_dir(root)
        .output()
        .expect("run git ls-files");
    assert!(out.status.success(), "git ls-files failed");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| root.join(l))
        .collect()
}

#[test]
fn table_covers_every_workspace_member() {
    let root = workspace_root();
    let mut problems = Vec::new();
    let members: BTreeSet<String> = workspace_guards::workspace_members()
        .iter()
        .map(|m| root.join(m).join("Cargo.toml").display().to_string())
        .collect();
    let read: BTreeSet<String> = workspaces()["."]
        .manifests
        .iter()
        .map(|p| p.display().to_string())
        .filter(|p| *p != root.join("Cargo.toml").display().to_string())
        .collect();
    for m in members.symmetric_difference(&read) {
        problems.push(format!(
            "dep_policy: `{m}` is a member in one of Cargo.toml and cargo metadata but not the other"
        ));
    }
    let covered: BTreeSet<PathBuf> = workspaces()
        .values()
        .flat_map(|ws| ws.manifests.iter().cloned())
        .chain(WORKSPACES.iter().map(|w| root.join(w).join("Cargo.toml")))
        .collect();
    let exempt: Vec<PathBuf> = WORKSPACES_EXEMPT
        .iter()
        .map(|(w, _)| root.join(w).join("Cargo.toml"))
        .collect();
    for (w, reason) in WORKSPACES_EXEMPT {
        if !root.join(w).join("Cargo.toml").is_file() {
            problems.push(format!(
                "dep_policy: exempt workspace `{w}` ({reason}) has no Cargo.toml; delete the entry"
            ));
        }
    }
    for manifest in tracked_manifests(&root) {
        let in_private = manifest.starts_with(root.join("private"));
        if !in_private && !covered.contains(&manifest) && !exempt.contains(&manifest) {
            problems.push(format!(
                "dep_policy: `{}` belongs to no workspace in WORKSPACES; add its workspace root there or to WORKSPACES_EXEMPT with a reason",
                manifest.display()
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn synthetic() -> Value {
    serde_json::json!({
        "packages": [
            {
                "name": "probe-a",
                "manifest_path": "/w/probe-a/Cargo.toml",
                "dependencies": [
                    { "name": "chrono", "kind": null, "rename": null },
                    { "name": "quick-xml", "kind": null, "rename": null },
                    { "name": "xml-rs", "kind": "dev", "rename": null },
                    { "name": "time", "kind": null, "rename": "clock" },
                    { "name": "evil-local", "kind": null, "rename": null, "path": "/w/evil-local" },
                    { "name": "probe-b", "kind": null, "rename": null }
                ]
            }
        ]
    })
}

const TEST_BANNED: &[Banned] = &[
    Banned {
        krate: "chrono",
        instead: "jiff",
    },
    Banned {
        krate: "time",
        instead: "jiff",
    },
];

const TEST_PURPOSES: &[Purpose] = &[
    Purpose {
        name: "XML",
        single: true,
        crates: &["quick-xml"],
    },
    Purpose {
        name: "XML (bad table)",
        single: false,
        crates: &["quick-xml", "xml-rs"],
    },
];

#[test]
fn detector_reads_real_names_and_skips_internal_deps() {
    let internal: BTreeSet<String> = ["probe-b".to_string()].into();
    let deps = external_deps(&synthetic(), &internal);
    let names: Vec<&str> = deps.iter().map(|d| d.krate.as_str()).collect();
    assert_eq!(names, ["chrono", "quick-xml", "time", "xml-rs"]);
    let banned = banned_problems(&deps, TEST_BANNED, &[]).join("\n");
    assert!(banned.contains("banned `chrono`"), "{banned}");
    assert!(banned.contains("banned `time`"), "{banned}");
    let exempted = banned_problems(
        &deps,
        TEST_BANNED,
        &[Exempt {
            member: "probe-a",
            krate: "chrono",
            reason: "test",
        }],
    )
    .join("\n");
    assert!(!exempted.contains("chrono"), "{exempted}");
    assert!(exempted.contains("banned `time`"), "{exempted}");
}

#[test]
fn detector_flags_unlisted_twice_listed_and_second_single_crate() {
    let deps = external_deps(&synthetic(), &BTreeSet::new());
    let purposes = purpose_problems(&deps, TEST_PURPOSES, &[]).join("\n");
    assert!(
        purposes.contains("`chrono` (used by probe-a (normal))"),
        "{purposes}"
    );
    assert!(
        purposes.contains("`quick-xml`") && purposes.contains("several purposes"),
        "{purposes}"
    );
    assert!(!purposes.contains("`evil-local`"), "{purposes}");
    let single_xml = [Purpose {
        name: "XML",
        single: true,
        crates: &["quick-xml", "xml-rs"],
    }];
    let single = single_problems(&deps, &single_xml, &[]).join("\n");
    assert!(
        single.contains("purpose `XML` is served by [\"quick-xml\", \"xml-rs\"]"),
        "{single}"
    );
    assert!(single.contains("one crate per purpose"), "{single}");
}

#[test]
fn ban_glob_matches_prefix_only() {
    assert!(matches("crc*", "crc32fast"));
    assert!(matches("crc*", "crc"));
    assert!(!matches("crc*", "rcrc"));
    assert!(matches("rand", "rand"));
    assert!(!matches("rand", "rand_core"));
}
