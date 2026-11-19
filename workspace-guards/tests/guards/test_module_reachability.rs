//! Tripwire: every `.rs` under a test binary's own directory (`tests/<bin>/`)
//! is reachable from that binary's `main.rs` through `mod ...;` declarations or
//! an `include!`.
//!
//! A file the binary cannot see is dead: it never compiles, so its `#[test]`s
//! never run. `broadcast-client/tests/it/tshttp/` sat like that for three
//! breaking commits (audit 23, finding #3150) while its policy-level tests
//! silently stopped protecting anything.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use workspace_guards::modules::{Declaration, declarations, module_base, path_base, resolve};
use workspace_guards::workspace_members;

/// Paths a file flashes in with `include!("...")`, relative to its own
/// directory; the included file is compiled, so it must count as reachable.
fn included_files(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|raw| {
            let rest = raw.trim().strip_prefix("include!(")?;
            let (_, rest) = rest.split_once('"')?;
            Some(rest.split_once('"')?.0.to_string())
        })
        .collect()
}

fn reachable_from(file: &Path, seen: &mut BTreeSet<PathBuf>) {
    let Ok(file) = fs::canonicalize(file) else {
        return;
    };
    if !seen.insert(file.clone()) {
        return;
    }
    let Ok(text) = fs::read_to_string(&file) else {
        return;
    };
    for rel in included_files(&text) {
        reachable_from(&path_base(&file).join(rel), seen);
    }
    for Declaration { path, name, .. } in declarations(&text) {
        let candidates = match path {
            Some(rel) => resolve(&path_base(&file), &rel),
            None => resolve(&module_base(&file), &name),
        };
        for candidate in candidates {
            if candidate.is_file() {
                reachable_from(&candidate, seen);
                break;
            }
        }
    }
}

#[test]
fn every_test_module_is_reachable_from_its_binary_main() {
    let mut problems = Vec::new();
    for member in workspace_members() {
        let tests = workspace_guards::crate_dir(&member).join("tests");
        let Ok(entries) = fs::read_dir(&tests) else {
            continue;
        };
        for entry in entries.flatten() {
            let bin_dir = entry.path();
            let main = bin_dir.join("main.rs");
            if !bin_dir.is_dir() || !main.is_file() {
                continue;
            }
            let mut seen = BTreeSet::new();
            reachable_from(&main, &mut seen);
            for file in workspace_guards::rs_files(&bin_dir) {
                let canonical = fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
                if !seen.contains(&canonical) {
                    let rel = file
                        .strip_prefix(workspace_guards::workspace_root())
                        .unwrap_or(&file);
                    problems.push(format!(
                        "{}: `{}` is not reachable from `{}/main.rs` (add a `mod` declaration)",
                        member,
                        rel.display(),
                        bin_dir
                            .strip_prefix(workspace_guards::workspace_root())
                            .unwrap_or(&bin_dir)
                            .display()
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn declarations_carry_their_path_attribute_and_skip_comments() {
    let text = "\
// mod dead;
#[cfg(feature = \"x\")]
mod plain;
#[path = \"support/helper.rs\"]
mod helper;
pub(crate) mod inner;
";
    assert_eq!(
        declarations(text),
        vec![
            Declaration {
                test_only: false,
                path: None,
                name: "plain".to_string(),
            },
            Declaration {
                test_only: false,
                path: Some("support/helper.rs".to_string()),
                name: "helper".to_string(),
            },
            Declaration {
                test_only: false,
                path: None,
                name: "inner".to_string(),
            },
        ]
    );
}

#[test]
fn a_test_only_declaration_is_marked_and_a_non_mod_rs_file_looks_beside_it() {
    assert_eq!(
        declarations("#[cfg(test)]\nmod tests;"),
        vec![Declaration {
            test_only: true,
            path: None,
            name: "tests".to_string(),
        }]
    );
    let file = Path::new("/crate/tests/it/common/checks.rs");
    assert_eq!(
        module_base(file),
        Path::new("/crate/tests/it/common/checks")
    );
    assert_eq!(path_base(file), Path::new("/crate/tests/it/common"));
    assert_eq!(
        module_base(Path::new("/crate/tests/it/main.rs")),
        Path::new("/crate/tests/it")
    );
    assert_eq!(
        module_base(Path::new("/crate/tests/it/common/mod.rs")),
        Path::new("/crate/tests/it/common")
    );
}

#[test]
fn test_only_modules_are_left_out_of_the_guarded_source() {
    let files = workspace_guards::src_files("broadcast-client");
    let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
    assert!(
        !names.iter().any(|p| p.contains("tshttp/tests/")),
        "{names:?}"
    );
    assert!(names.contains(&"src/tshttp/policy.rs"), "{names:?}");
}
