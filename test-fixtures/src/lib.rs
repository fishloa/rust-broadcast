//! Shared fixture loader for workspace test suites (unpublished).
//!
//! 448 test files resolve the workspace `fixtures/` directory with
//! `env!("CARGO_MANIFEST_DIR")` and ~65 of them hand-wrote their own
//! `fixture()` / `read_fixture()` / `fixture_path()` around it (each with a
//! slightly different panic message, relative-path shape and `Vec<u8>` vs
//! `String` flavour). The path arithmetic is the one thing that must be right
//! and it belongs in one place: because `env!("CARGO_MANIFEST_DIR")` is
//! evaluated HERE, in this crate, [`root`] resolves to `<workspace>/fixtures`
//! regardless of which crate's test calls it, so a caller can neither get the
//! `../` depth wrong nor silently fall back to a relative path under a
//! different working directory.
//!
//! A missing fixture PANICS with the full resolved path (the project rule:
//! never skip on a missing committed fixture - a skip would make the test
//! vacuous). The optional, non-redistributable corpus lives in the private
//! submodule and is reached through [`private`], which is `None` when the
//! submodule is not initialised so each caller makes the skip decision
//! explicitly. Setting `SKIP_PRIVATE_FIXTURES=1` disables that tier even when
//! a checkout exists (and it wins over `REQUIRE_PRIVATE_FIXTURES=1`).
//!
//! `publish = false` and no workspace-sibling dependencies: a path-only
//! dev-dependency on it never appears in a published manifest (same pattern as
//! `test-bounded` / `test-alloc`).

use std::path::{Path, PathBuf};

/// The workspace `fixtures/` directory (committed, redistributable fixtures).
#[must_use]
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

/// `fixtures/<rel>` (`rel` uses `/` separators, e.g. `"ts/m6-single.ts"`).
#[must_use]
pub fn path(rel: &str) -> PathBuf {
    root().join(rel)
}

/// The bytes of `fixtures/<rel>`.
///
/// # Panics
/// With the full resolved path if the fixture cannot be read - a committed
/// fixture must exist, a skip would make the calling test vacuous.
#[must_use]
pub fn read(rel: &str) -> Vec<u8> {
    let p = path(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {} must be committed: {e}", p.display()))
}

/// `fixtures/<rel>` as UTF-8 text.
///
/// # Panics
/// With the full resolved path if the fixture cannot be read or is not UTF-8.
#[must_use]
pub fn read_to_string(rel: &str) -> String {
    let p = path(rel);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("fixture {} must be committed: {e}", p.display()))
}

/// Environment variable that points at a checkout of the private repo (the
/// directory that contains `fixtures/`), e.g. from a CI deploy-key job or a
/// worktree whose own `private/` is not initialised.
pub const PRIVATE_ENV: &str = "RUST_BROADCAST_PRIVATE";

/// Environment variable CI sets to `1` when it has the private-repo deploy key:
/// a private fixture that cannot be found is then a hard failure, never a
/// (vacuous) skip.
pub const REQUIRE_PRIVATE_ENV: &str = "REQUIRE_PRIVATE_FIXTURES";

/// Environment variable that disables the private tier when it is exactly `1`:
/// [`private_root`] then resolves to `None` (a loud skip) even when a checkout
/// exists, and even when [`REQUIRE_PRIVATE_ENV`] is set - the skip wins. Set
/// per job in CI, or locally to run only the redistributable suite.
pub const SKIP_PRIVATE_ENV: &str = "SKIP_PRIVATE_FIXTURES";

/// `true` when `v` is exactly `1` (the value of [`REQUIRE_PRIVATE_ENV`]).
fn flag_set(v: Option<&std::ffi::OsStr>) -> bool {
    v.is_some_and(|v| v == "1")
}

/// [`private_root`]'s body with the two environment flags passed in, so the
/// precedence (skip over require) is testable without touching process-global
/// env vars.
fn private_root_with(checkout: Option<PathBuf>, skip: bool) -> Option<PathBuf> {
    if skip {
        return None;
    }
    checkout
}

/// The workspace root (the directory holding `Cargo.toml`, `fixtures/`, `private/`).
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Root of the private fixtures repo (`.../private`), if it is checked out.
///
/// Resolution order: `$RUST_BROADCAST_PRIVATE`; `<workspace>/private`; and, for
/// a linked git worktree (whose own `private/` is an empty directory), the
/// `private/` of the main checkout the worktree belongs to. `None` means the
/// private repo is simply not available (public / CI clones).
///
/// `None` also when [`SKIP_PRIVATE_ENV`] is exactly `1`, whatever exists and
/// whatever [`REQUIRE_PRIVATE_ENV`] says.
#[must_use]
pub fn private_root() -> Option<PathBuf> {
    let skip = flag_set(std::env::var_os(SKIP_PRIVATE_ENV).as_deref());
    private_root_with(private_checkout(), skip)
}

/// The private checkout without the [`SKIP_PRIVATE_ENV`] override.
fn private_checkout() -> Option<PathBuf> {
    let has_fixtures = |p: &Path| p.join("fixtures").is_dir();
    if let Some(v) = std::env::var_os(PRIVATE_ENV) {
        let p = PathBuf::from(v);
        if has_fixtures(&p) {
            return Some(p);
        }
    }
    let ws = workspace();
    let own = ws.join("private");
    if has_fixtures(&own) {
        return Some(own);
    }
    // Linked worktree: `.git` is a file `gitdir: <main>/.git/worktrees/<name>`.
    let dot_git = std::fs::read_to_string(ws.join(".git")).ok()?;
    let gitdir = PathBuf::from(dot_git.strip_prefix("gitdir:")?.trim());
    let main = gitdir.ancestors().nth(3)?; // <main>/.git/worktrees/<name> -> <main>
    let p = main.join("private");
    has_fixtures(&p).then_some(p)
}

/// `private/fixtures/<rel>` in the private repo, or `None` when the repo is not
/// checked out or lacks that entry.
#[must_use]
pub fn private(rel: &str) -> Option<PathBuf> {
    let p = private_root()?.join("fixtures").join(rel);
    p.exists().then_some(p)
}

/// Whether `dir` is the checked-out private repo (resolved by [`private_root`]).
///
/// A "committed fixtures only" walk of the workspace tree must skip it by
/// *identity*, not by directory name: the private checkout is `private/` in a
/// developer clone but `private-fixtures/` in CI (and wherever
/// [`PRIVATE_ENV`] points), and a name list misses the second.
#[must_use]
pub fn is_private_checkout(dir: &Path) -> bool {
    let Some(root) = private_root() else {
        return false;
    };
    match (dir.canonicalize(), root.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Directory of the pinned BSD-2 `tsduck/tsduck-test` clone, if present. The
/// clone is fetched on demand by `tools/fetch-test-streams.sh tsduck-test` into
/// `.test-streams/tsduck-test` (repository URL and pinned commit are recorded
/// in that script); the files the tests actually reference are also mirrored
/// under `private/fixtures/tsduck/`.
#[must_use]
pub fn tsduck_dir() -> Option<PathBuf> {
    let clone = workspace().join(".test-streams/tsduck-test");
    if clone.is_dir() {
        return Some(clone);
    }
    private("tsduck")
}

/// Where a located fixture came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Committed, redistributable `fixtures/`.
    Public,
    /// `private/fixtures/` (private repo).
    Private,
    /// Legacy fetch-on-demand `.test-streams/` directory.
    Legacy,
}

/// Locate `rel` in, in order: the committed `fixtures/`, `private/fixtures/`,
/// and (legacy fallback) `.test-streams/`.
#[must_use]
pub fn find(rel: &str) -> Option<(PathBuf, Source)> {
    let public = path(rel);
    if public.exists() {
        return Some((public, Source::Public));
    }
    if let Some(p) = private(rel) {
        return Some((p, Source::Private));
    }
    let legacy = workspace().join(".test-streams").join(rel);
    legacy.exists().then_some((legacy, Source::Legacy))
}

/// The single entry point for tests that need a non-redistributable fixture
/// (a real broadcast capture etc.): `Some(path)` when found, `None` after a
/// loud `SKIPPED` line when the private repo is **not checked out**
/// (public / CI clones).
///
/// # Panics
/// If the private repo *is* checked out (or the entry is otherwise expected)
/// but `rel` is in none of the three locations: a missing private fixture in a
/// checkout that has the private repo is a repository error, never a skip.
#[must_use]
pub fn require(rel: &str, who: &str) -> Option<PathBuf> {
    if let Some((p, _)) = find(rel) {
        return Some(p);
    }
    if flag_set(std::env::var_os(SKIP_PRIVATE_ENV).as_deref()) {
        eprintln!(
            "{who}: SKIPPED - private fixture `{rel}`: the private tier is disabled by \
             {SKIP_PRIVATE_ENV}=1"
        );
        return None;
    }
    match private_root() {
        None if flag_set(std::env::var_os(REQUIRE_PRIVATE_ENV).as_deref()) => panic!(
            "{who}: private fixture `{rel}` unavailable but {REQUIRE_PRIVATE_ENV}=1 (CI has the \
             deploy key, so the private checkout must exist - a skip here would hide the test)"
        ),
        None => {
            eprintln!(
                "{who}: SKIPPED - private fixture `{rel}` unavailable: the private repo is not \
                 checked out (git submodule update --init private, or set {PRIVATE_ENV})"
            );
            None
        }
        Some(root) => panic!(
            "{who}: fixture `{rel}` is in none of fixtures/, {}/fixtures/, .test-streams/",
            root.display()
        ),
    }
}

/// Locate `rel` (e.g. `input/test-002.ts`, `reference/test-147/test-147.ts`) in
/// the pinned `tsduck-test` clone (`.test-streams/tsduck-test`) or the
/// `private/fixtures/tsduck/` mirror. Same skip/panic contract as [`require`].
///
/// # Panics
/// If the private repo is checked out but `rel` is in neither location.
#[must_use]
pub fn tsduck(rel: &str, who: &str) -> Option<PathBuf> {
    let clone = workspace().join(".test-streams/tsduck-test").join(rel);
    if clone.exists() {
        return Some(clone);
    }
    require(&format!("tsduck/{rel}"), who)
}

/// Bytes of [`require`]`(rel, who)`, or `None` on a loud skip.
#[must_use]
pub fn require_bytes(rel: &str, who: &str) -> Option<Vec<u8>> {
    let p = require(rel, who)?;
    Some(std::fs::read(&p).unwrap_or_else(|e| panic!("{who}: read {}: {e}", p.display())))
}

fn walk_files(d: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(d) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_files(&p, exts, out);
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| exts.contains(&x))
        {
            out.push(p);
        }
    }
}

/// Every regular file under `private/fixtures/<dir>` (recursive, sorted) with
/// one of the given extensions; empty when the private repo is not checked out.
#[must_use]
pub fn private_files(dir: &str, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = private(dir) {
        walk_files(&d, exts, &mut out);
    }
    out.sort();
    out
}

/// The whole real-capture corpus for the scan/survey tests: every file with
/// one of `exts` under `private/fixtures/` plus, if present, the fetched
/// `.test-streams/` directory (which holds the pinned `tsduck-test` clone).
/// Sorted; empty when neither is available.
#[must_use]
pub fn corpus(exts: &[&str]) -> Vec<PathBuf> {
    let mut out = private_files("", exts);
    walk_files(&workspace().join(".test-streams"), exts, &mut out);
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_the_workspace_fixtures_directory() {
        assert!(root().is_dir(), "{}", root().display());
        assert!(root().join("ts").is_dir());
    }

    #[test]
    fn read_returns_the_real_bytes_of_a_committed_fixture() {
        // Every transport-stream fixture starts with the 0x47 sync byte.
        let ts = read("ts/m6-single.ts");
        assert_eq!(ts[0], 0x47);
        assert_eq!(ts.len() % 188, 0);
        assert_eq!(ts, std::fs::read(path("ts/m6-single.ts")).unwrap());
    }

    #[test]
    fn read_to_string_decodes_a_text_fixture() {
        let s = read_to_string("hls/bad-extinf.m3u8");
        assert!(!s.is_empty());
    }

    #[test]
    #[should_panic(expected = "must be committed")]
    fn a_missing_fixture_panics_with_the_resolved_path() {
        let _ = read("ts/no-such-fixture.ts");
    }

    #[test]
    fn private_is_none_for_an_absent_entry() {
        assert!(private("definitely/not/here.bin").is_none());
    }

    #[test]
    fn find_prefers_the_committed_fixture() {
        let (p, src) = find("ts/m6-single.ts").unwrap();
        assert_eq!(src, Source::Public);
        assert_eq!(p, path("ts/m6-single.ts"));
    }

    #[test]
    fn find_is_none_for_an_entry_in_no_location() {
        assert!(find("definitely/not/here.bin").is_none());
    }

    #[test]
    fn require_skips_loudly_only_without_the_private_repo() {
        if private_root().is_none() {
            assert!(require("definitely/not/here.bin", "unit").is_none());
        }
    }

    #[test]
    fn skip_private_flag_is_exactly_one() {
        use std::ffi::OsStr;
        assert!(flag_set(Some(OsStr::new("1"))));
        assert!(!flag_set(Some(OsStr::new("true"))));
        assert!(!flag_set(None));
    }

    #[test]
    fn skip_private_disables_a_present_checkout_and_wins_over_require() {
        let present = Some(PathBuf::from("/private"));
        assert_eq!(private_root_with(present.clone(), false), present);
        assert_eq!(private_root_with(present, true), None);
        assert_eq!(private_root_with(None, true), None);
        assert_eq!(private_root_with(None, false), None);
    }

    #[test]
    fn skip_private_env_disables_the_private_tier() {
        // Same thread-safety rule as the other tests here: assert the unset
        // case only when the variable is unset, and the disabled case always
        // (the helper is a pure function of the flag).
        if std::env::var_os(SKIP_PRIVATE_ENV).is_none() {
            assert_eq!(private_root(), private_checkout());
        } else if flag_set(std::env::var_os(SKIP_PRIVATE_ENV).as_deref()) {
            assert_eq!(private_root(), None);
            assert!(find("definitely/not/here.bin").is_none());
        }
    }

    #[test]
    fn require_private_flag_is_exactly_one() {
        use std::ffi::OsStr;
        assert!(flag_set(Some(OsStr::new("1"))));
        assert!(!flag_set(Some(OsStr::new("0"))));
        assert!(!flag_set(Some(OsStr::new("true"))));
        assert!(!flag_set(Some(OsStr::new(""))));
        assert!(!flag_set(None));
    }

    #[test]
    fn require_panics_when_private_is_present_but_the_entry_is_missing() {
        if private_root().is_none() {
            return; // nothing to assert without the private repo
        }
        let r = std::panic::catch_unwind(|| require("definitely/not/here.bin", "unit"));
        assert!(
            r.is_err(),
            "a missing entry in a checkout WITH private/ must panic"
        );
    }
}
