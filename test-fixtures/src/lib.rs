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
//! explicitly.
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

/// `private/fixtures/<rel>` in the private submodule, or `None` when the
/// submodule is not initialised (public / CI clones), so the caller decides
/// whether that is a loud skip.
#[must_use]
pub fn private(rel: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../private/fixtures")
        .join(rel);
    p.exists().then_some(p)
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
}
