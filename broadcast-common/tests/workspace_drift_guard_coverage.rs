//! Drift-guard of the drift guards (issue #806, part 4).
//!
//! `tests/label_coverage.rs` (the #204 Display convention) and
//! `tests/non_exhaustive_coverage.rs` (`#[non_exhaustive]`) are per-crate
//! conventions: nothing forces a *new* workspace member to carry either one.
//! That gap already let a real defect through once (`media_plane::TapItem`
//! shipped without `#[non_exhaustive]` while its closest analogues had it —
//! caught by hand during a pre-release audit, not by CI).
//!
//! This test closes the gap one level up: it walks every crate listed in the
//! root `Cargo.toml`'s `[workspace] members`, and for each one that defines a
//! `src/lib.rs` (i.e. has a public API surface for either guard to police),
//! asserts it carries both `tests/label_coverage.rs` and
//! `tests/non_exhaustive_coverage.rs` -- unless it is named in one of the
//! EXEMPT lists below with a reason. A crate added to the workspace with
//! neither guard file nor a recorded exemption fails this test, so the *net*
//! itself cannot silently develop a hole the way the per-crate gap did.
//!
//! Binaries with no library target (no public API, hence nothing for either
//! guard to police) are recorded in `BINARY_ONLY` instead.

use std::fs;
use std::path::{Path, PathBuf};

/// Workspace members that define no `src/lib.rs` -- pure binaries with no
/// public API surface for either drift guard to police.
const BINARY_ONLY: &[&str] = &["dvb-tools", "multimux-cli"];

/// Library crates exempt from `tests/label_coverage.rs` (the #204 Display
/// convention), with a one-line reason. The only legitimate reason is "no
/// public enums in `src/`" -- anything else means the guard belongs there
/// instead, or the crate's Display convention is genuinely handled another
/// way.
const LABEL_COVERAGE_EXEMPT: &[(&str, &str)] = &[
    (
        "dvb-stream",
        "no public enums in src/ (async stream adapters + ResyncStats struct only)",
    ),
    (
        "dvb-csa",
        "only public enum is Error (exempt from #204 labels)",
    ),
];

/// Library crates exempt from `tests/non_exhaustive_coverage.rs`, with a
/// one-line reason. The only legitimate reason is "no public enums in
/// `src/`".
const NON_EXHAUSTIVE_EXEMPT: &[(&str, &str)] = &[
    (
        "dvb-stream",
        "no public enums in src/ (async stream adapters + ResyncStats struct only)",
    ),
    (
        "dvb-csa",
        "only public enum is Error (exempt from #[non_exhaustive] guard)",
    ),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("broadcast-common has a parent dir")
        .to_path_buf()
}

/// Parses the single-line `members = [...]` array out of the root
/// `Cargo.toml`. Deliberately minimal (no toml crate dependency): the array
/// is a flat list of quoted strings on one line, as committed today.
fn members(root: &Path) -> Vec<String> {
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read root Cargo.toml");
    let line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("members"))
        .expect("root Cargo.toml has a `members = [...]` line");
    let start = line.find('[').expect("members line has `[`") + 1;
    let end = line.rfind(']').expect("members line has `]`");
    line[start..end]
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn every_library_crate_has_both_drift_guards_or_a_recorded_exemption() {
    let root = workspace_root();
    let all_members = members(&root);

    // Sanity-check the exemption lists themselves: an exemption for a crate
    // that no longer exists (renamed/removed) would silently stop meaning
    // anything.
    for (name, _) in LABEL_COVERAGE_EXEMPT.iter().chain(NON_EXHAUSTIVE_EXEMPT) {
        assert!(
            all_members.iter().any(|m| m == name),
            "exemption list names `{name}`, which is not a current workspace member -- \
             remove the stale entry"
        );
    }
    for name in BINARY_ONLY {
        assert!(
            all_members.iter().any(|m| m == name),
            "BINARY_ONLY names `{name}`, which is not a current workspace member -- \
             remove the stale entry"
        );
    }

    let mut missing_label = Vec::new();
    let mut missing_non_exhaustive = Vec::new();
    let mut unexpectedly_binary = Vec::new();

    for member in &all_members {
        let crate_dir = root.join(member);
        let has_lib = crate_dir.join("src").join("lib.rs").is_file();

        if !has_lib {
            if !BINARY_ONLY.contains(&member.as_str()) {
                unexpectedly_binary.push(member.clone());
            }
            continue;
        }

        let has_label_guard = crate_dir.join("tests").join("label_coverage.rs").is_file();
        let label_exempt = LABEL_COVERAGE_EXEMPT.iter().any(|(c, _)| c == member);
        if !has_label_guard && !label_exempt {
            missing_label.push(member.clone());
        }

        let has_ne_guard = crate_dir
            .join("tests")
            .join("non_exhaustive_coverage.rs")
            .is_file();
        let ne_exempt = NON_EXHAUSTIVE_EXEMPT.iter().any(|(c, _)| c == member);
        if !has_ne_guard && !ne_exempt {
            missing_non_exhaustive.push(member.clone());
        }
    }

    assert!(
        unexpectedly_binary.is_empty(),
        "workspace member(s) have no src/lib.rs but are not in BINARY_ONLY: \
         {unexpectedly_binary:?} -- add them there with a reason, or add the missing lib.rs"
    );
    assert!(
        missing_label.is_empty(),
        "library crate(s) missing tests/label_coverage.rs and not in \
         LABEL_COVERAGE_EXEMPT: {missing_label:?}"
    );
    assert!(
        missing_non_exhaustive.is_empty(),
        "library crate(s) missing tests/non_exhaustive_coverage.rs and not in \
         NON_EXHAUSTIVE_EXEMPT: {missing_non_exhaustive:?}"
    );
}
