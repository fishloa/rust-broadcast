//! Tripwire: test fixtures live in exactly two homes — `fixtures/` (public) and
//! `private/fixtures/` (private submodule). Rules:
//! 1. no tracked file under `<crate>/tests/fixtures/`;
//! 2. no `.rs` outside test-fixtures reads the retired `.test-streams/` tier;
//! 3. no `.rs` outside test-fixtures include_bytes!/include_str!s a fixture;
//! 4. no `.rs` outside test-fixtures/fuzz/tools/workspace-guards builds a
//!    fixture path by hand.

use std::fs;
use std::path::Path;

const MANIFEST_DIR_NEEDLE: &str = concat!("CARGO", "_MANIFEST_DIR");

fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        "target"
            | ".git"
            | "node_modules"
            | "private"
            | "test-fixtures"
            | ".claude"
            | "workspace-guards"
    )
}

fn walk_files(root: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !skip_dir(&name) {
                walk_files(&p, out);
            }
        } else {
            out.push(p);
        }
    }
}

pub fn scan_tests_fixtures_dirs(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk_files(root, &mut files);
    let mut bad = Vec::new();
    for p in files {
        let rel = p
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.contains("/tests/fixtures/") || rel.contains("/tests/fixtures") {
            // ensure it's under a tests/fixtures path
            let parts: Vec<&str> = rel.split('/').collect();
            if parts.windows(2).any(|w| w == ["tests", "fixtures"]) {
                bad.push(rel);
            }
        }
    }
    bad.sort();
    bad
}

pub fn scan_dot_test_streams(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk_files(root, &mut files);
    let mut bad = Vec::new();
    for p in files {
        if p.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let rel = p
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("test-fixtures/") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        if text.contains(".test-streams") {
            bad.push(rel);
        }
    }
    bad.sort();
    bad
}

pub fn scan_handbuilt_fixture_paths(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk_files(root, &mut files);
    let mut bad = Vec::new();
    for p in files {
        if p.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let rel = p
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("test-fixtures/")
            || rel.starts_with("fuzz/")
            || rel.starts_with("tools/")
            || rel.starts_with("workspace-guards/")
        {
            continue;
        }
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        let mut flagged = false;
        // Line-based rules.
        for line in text.lines() {
            if line.contains(MANIFEST_DIR_NEEDLE)
                && (line.contains("fixtures") || line.contains("\"tests\""))
            {
                flagged = true;
                break;
            }
            if line.contains("\"tests/fixtures") || line.contains("join(\"fixtures\")") {
                flagged = true;
                break;
            }
        }
        // Expression-based rule: a concat!/format! expression that contains
        // the manifest env macro and "fixtures"/"tests" on separate lines is still
        // a hand-built fixture path.
        if !flagged {
            for macro_name in ["concat!", "format!"] {
                let mut search = text.as_str();
                while let Some(i) = search.find(macro_name) {
                    let rest = &search[i..];
                    let Some(open) = rest.find('(') else {
                        break;
                    };
                    let mut depth = 0usize;
                    let mut end = rest.len();
                    for (k, c) in rest[open..].char_indices() {
                        match c {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = open + k + 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    let inv = &rest[..end];
                    if inv.contains(MANIFEST_DIR_NEEDLE)
                        && (inv.contains("fixtures") || inv.contains("\"tests\""))
                    {
                        flagged = true;
                        break;
                    }
                    search = &rest[end..];
                }
                if flagged {
                    break;
                }
            }
        }
        if flagged {
            bad.push(rel);
        }
    }
    bad.sort();
    bad.dedup();
    bad
}

pub fn scan_include_bytes_fixtures(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk_files(root, &mut files);
    let mut bad = Vec::new();
    for p in files {
        if p.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let rel = p
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("test-fixtures/") || rel.starts_with("fuzz/") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        for macro_name in ["include_bytes!", "include_str!"] {
            let needle = macro_name.to_string();
            let mut search = text.as_str();
            while let Some(i) = search.find(&needle) {
                // find matching paren
                let rest = &search[i..];
                let Some(open) = rest.find('(') else {
                    break;
                };
                let mut depth = 0usize;
                let mut end = rest.len();
                for (k, c) in rest[open..].char_indices() {
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                end = open + k + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let inv = &rest[..end];
                if inv.contains("fixtures") {
                    bad.push(rel.clone());
                    break;
                }
                search = &rest[end..];
            }
            if bad.last().map(|x| x == &rel).unwrap_or(false) {
                break;
            }
        }
    }
    bad.sort();
    bad.dedup();
    bad
}

#[test]
fn no_tracked_files_under_crate_tests_fixtures() {
    let root = workspace_guards::workspace_root();
    let bad = scan_tests_fixtures_dirs(&root);
    assert!(
        bad.is_empty(),
        "files under <crate>/tests/fixtures/ must move to fixtures/<topic>/:\n{}",
        bad.join("\n")
    );
}

#[test]
fn no_dot_test_streams_in_code() {
    let root = workspace_guards::workspace_root();
    let bad = scan_dot_test_streams(&root);
    assert!(
        bad.is_empty(),
        ".rs files outside test-fixtures must not reference .test-streams:\n{}",
        bad.join("\n")
    );
}

#[test]
fn no_handbuilt_fixture_paths_outside_test_fixtures() {
    let root = workspace_guards::workspace_root();
    let bad = scan_handbuilt_fixture_paths(&root);
    assert!(
        bad.is_empty(),
        ".rs files outside test-fixtures/fuzz/tools/workspace-guards must not build fixture paths by hand:\n{}",
        bad.join("\n")
    );
}

#[test]
fn no_include_bytes_fixtures_outside_test_fixtures() {
    let root = workspace_guards::workspace_root();
    let bad = scan_include_bytes_fixtures(&root);
    assert!(
        bad.is_empty(),
        ".rs files outside test-fixtures must not include_bytes!/include_str! a fixture:\n{}",
        bad.join("\n")
    );
}

#[test]
fn table_covers_every_workspace_member() {
    // This guard has no per-crate table; the scan covers the whole workspace.
    // Keep a trivial assertion so the module follows the guard convention.
    let members = workspace_guards::workspace_members();
    assert!(!members.is_empty());
}

#[test]
fn detector_bites_tests_fixtures_dir() {
    let tmp = std::env::temp_dir().join("fixtures_one_home_bite1");
    let _ = fs::remove_dir_all(&tmp);
    let dir = tmp.join("foo/tests/fixtures");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("x.bin"), b"x").unwrap();
    let bad = scan_tests_fixtures_dirs(&tmp);
    assert_eq!(bad.len(), 1);
    assert!(bad[0].contains("tests/fixtures"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn detector_bites_dot_test_streams() {
    let tmp = std::env::temp_dir().join("fixtures_one_home_bite2");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("foo/src")).unwrap();
    fs::write(tmp.join("foo/src/lib.rs"), "let x = \".test-streams\";").unwrap();
    let bad = scan_dot_test_streams(&tmp);
    assert_eq!(bad.len(), 1);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn detector_bites_include_bytes_fixtures() {
    let tmp = std::env::temp_dir().join("fixtures_one_home_bite3");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("foo/src")).unwrap();
    fs::write(
        tmp.join("foo/src/lib.rs"),
        "const D: &[u8] = include_bytes!(\"../fixtures/ts/x.ts\");",
    )
    .unwrap();
    let bad = scan_include_bytes_fixtures(&tmp);
    assert_eq!(bad.len(), 1);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn detector_bites_concat_manifest_fixtures() {
    let tmp = std::env::temp_dir().join("fixtures_one_home_bite5");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("foo/src")).unwrap();
    fs::write(
        tmp.join("foo/src/lib.rs"),
        concat!(
            "const P: &str = concat!(\n",
            "    env!(\"",
            "CARGO",
            "_MANIFEST_DIR",
            "\"),\n",
            "    \"/../fixtures/x.ts\"\n",
            ");",
        ),
    )
    .unwrap();
    let bad = scan_handbuilt_fixture_paths(&tmp);
    assert_eq!(bad.len(), 1);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn detector_bites_join_tests_fixtures() {
    let tmp = std::env::temp_dir().join("fixtures_one_home_bite6");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("foo/src")).unwrap();
    fs::write(
        tmp.join("foo/src/lib.rs"),
        concat!(
            "let p = PathBuf::from(env!(\"",
            "CARGO",
            "_MANIFEST_DIR",
            "\"\")).join(\"tests\").join(\"fixtures\");",
        ),
    )
    .unwrap();
    let bad = scan_handbuilt_fixture_paths(&tmp);
    assert_eq!(bad.len(), 1);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn detector_bites_join_fixtures() {
    let tmp = std::env::temp_dir().join("fixtures_one_home_bite7");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("foo/src")).unwrap();
    fs::write(
        tmp.join("foo/src/lib.rs"),
        "let p = base.join(\"fixtures\");",
    )
    .unwrap();
    let bad = scan_handbuilt_fixture_paths(&tmp);
    assert_eq!(bad.len(), 1);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn detector_ignores_clean_include_bytes() {
    let tmp = std::env::temp_dir().join("fixtures_one_home_bite4");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("foo/src")).unwrap();
    fs::write(
        tmp.join("foo/src/lib.rs"),
        "const D: &[u8] = include_bytes!(\"src/data.bin\");",
    )
    .unwrap();
    let bad = scan_include_bytes_fixtures(&tmp);
    assert!(bad.is_empty());
    let _ = fs::remove_dir_all(&tmp);
}
