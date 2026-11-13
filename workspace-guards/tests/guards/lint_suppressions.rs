//! Guard: no lint suppressions anywhere in the workspace. Scans every `.rs`
//! file of every workspace member (src, tests, examples, benches) plus the
//! `fuzz/` and `tools/` trees and fails on any `#[allow(`, `#![allow(`,
//! `#[expect(`, `#![expect(` or `#[cfg_attr(..., allow(...))]`. The only
//! permitted entries are `#[allow(unsafe_code)]` at the five audited sites,
//! each pinned by file plus the first non-attribute, non-comment line after
//! the attribute (or by file alone for an inner `#![allow(unsafe_code)]`),
//! and each must carry a `// SAFETY:` comment. Also catches two dead-code
//! cheats: a `#[used]` static, and `let _ = <bare path>;` at item level
//! inside a const/static block.

use workspace_guards::scan::blank_comments_and_strings;

pub struct Crate {
    pub name: &'static str,
    /// `(file suffix, first non-attribute non-comment line after the
    /// attribute, reason)`. An inner `#![allow(unsafe_code)]` is pinned by
    /// file alone: its second element is the empty string.
    pub allow: &'static [(&'static str, &'static str, &'static str)],
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub line: usize,
    pub kind: &'static str,
    pub inner: bool,
    pub first_line: String,
    pub text: String,
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && (s.chars().next().unwrap().is_ascii_alphabetic() || s.starts_with('_'))
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn net_braces(line: &str) -> i32 {
    let mut d = 0;
    for c in line.chars() {
        if c == '{' {
            d += 1;
        } else if c == '}' {
            d -= 1;
        }
    }
    d
}

fn next_code_line(text: &str, start: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let blank = blank_comments_and_strings(text);
    let blank_lines: Vec<&str> = blank.lines().collect();
    let mut j = start;
    while j < lines.len() {
        let b = blank_lines.get(j).map(|s| s.trim()).unwrap_or("");
        if b.is_empty() {
            j += 1;
            continue;
        }
        if b.starts_with("#[") || b.starts_with("#![") {
            j += 1;
            continue;
        }
        return lines[j].trim().to_string();
    }
    String::new()
}

fn is_static_item(line: &str) -> bool {
    let mut toks = line.split_whitespace();
    loop {
        match toks.next() {
            Some("pub") | Some("pub(crate)") | Some("pub(super)") => continue,
            Some(t) if t.starts_with("pub(") => continue,
            Some("unsafe") => continue,
            Some("static") => return true,
            _ => return false,
        }
    }
}

fn is_let_underscore_bare_path(line: &str) -> bool {
    let t = line.trim();
    if !t.ends_with(';') {
        return false;
    }
    let body = t.strip_suffix(';').unwrap().trim();
    let Some(rest) = body.strip_prefix("let _ =") else {
        return false;
    };
    let path = rest.trim().replace(' ', "");
    if path.is_empty() {
        return false;
    }
    path.split("::").all(is_ident)
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

fn scan_file(path: &str, text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let blank = blank_comments_and_strings(text);
    let orig_lines: Vec<&str> = text.lines().collect();
    let blank_lines: Vec<&str> = blank.lines().collect();
    let n = orig_lines.len().max(blank_lines.len());

    // Attribute scan.
    for i in 0..n {
        let bl = blank_lines.get(i).copied().unwrap_or("").trim();
        if bl.is_empty() {
            continue;
        }
        let inner = bl.contains("#![");
        let mut kind: Option<&'static str> = None;
        if bl.contains("cfg_attr") && bl.contains("allow(") {
            kind = Some("cfg_attr_allow");
        } else if bl.contains("allow(") && (bl.contains("#[allow(") || bl.contains("#![allow(")) {
            kind = Some("allow");
        } else if bl.contains("expect(") && (bl.contains("#[expect(") || bl.contains("#![expect("))
        {
            kind = Some("expect");
        }
        if let Some(k) = kind {
            // Only count actual attribute lines.
            if bl.starts_with("#[") || bl.starts_with("#![") {
                let fl = next_code_line(text, i + 1);
                findings.push(Finding {
                    line: i + 1,
                    kind: k,
                    inner,
                    first_line: fl,
                    text: orig_lines.get(i).copied().unwrap_or("").trim().to_string(),
                });
            }
        }
        // #[used] static
        if (bl.contains("#[used]") || bl.contains("#![used]"))
            && (bl.starts_with("#[") || bl.starts_with("#!["))
        {
            let fl = next_code_line(text, i + 1);
            if is_static_item(&fl) {
                findings.push(Finding {
                    line: i + 1,
                    kind: "used",
                    inner: bl.contains("#!["),
                    first_line: fl.clone(),
                    text: orig_lines.get(i).copied().unwrap_or("").trim().to_string(),
                });
            }
        }
    }

    // let _ = <bare path>; inside const/static blocks.
    let mut in_block = false;
    let mut depth = 0i32;
    for i in 0..n {
        let bl = blank_lines.get(i).copied().unwrap_or("").trim().to_string();
        if in_block {
            if is_let_underscore_bare_path(&bl) {
                findings.push(Finding {
                    line: i + 1,
                    kind: "let_underscore",
                    inner: false,
                    first_line: String::new(),
                    text: orig_lines.get(i).copied().unwrap_or("").trim().to_string(),
                });
            }
            depth += net_braces(&bl);
            if depth <= 0 {
                in_block = false;
                depth = 0;
            }
        } else if (bl.contains("const ") || bl.contains("static ")) && bl.contains('{') {
            in_block = true;
            depth = net_braces(&bl);
            if is_let_underscore_bare_path(&bl) {
                findings.push(Finding {
                    line: i + 1,
                    kind: "let_underscore",
                    inner: false,
                    first_line: String::new(),
                    text: orig_lines.get(i).copied().unwrap_or("").trim().to_string(),
                });
            }
            if depth <= 0 {
                in_block = false;
                depth = 0;
            }
        }
    }

    let _ = path;
    findings
}

pub fn hits_in(text: &str) -> Vec<(usize, String)> {
    scan_file("x", text)
        .into_iter()
        .map(|f| (f.line, f.text))
        .collect()
}

pub fn problems_in(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut used = vec![false; c.allow.len()];

    for (path, text) in files {
        for f in scan_file(path, text) {
            let is_unsafe_allow = f.text.contains("unsafe_code")
                && (f.kind == "allow" || f.kind == "expect" || f.kind == "cfg_attr_allow");

            let mut matched: Option<usize> = None;
            for (idx, (file, first, _reason)) in c.allow.iter().enumerate() {
                if !path.ends_with(file) {
                    continue;
                }
                if f.inner {
                    if first.is_empty() {
                        matched = Some(idx);
                        break;
                    }
                } else if *first == f.first_line {
                    matched = Some(idx);
                    break;
                }
            }

            if let Some(idx) = matched {
                if !is_unsafe_allow {
                    problems.push(format!(
                        "{}: {path}:{}: suppression in disguise: {}",
                        c.name, f.line, f.text
                    ));
                    continue;
                }
                used[idx] = true;
                if !text.contains("// SAFETY:") {
                    problems.push(format!(
                        "{}: {path}:{}: allow(unsafe_code) without SAFETY comment",
                        c.name, f.line
                    ));
                }
            } else {
                problems.push(format!(
                    "{}: {path}:{}: forbidden lint suppression: {}",
                    c.name, f.line, f.text
                ));
            }
        }
    }

    for (idx, (file, first, _reason)) in c.allow.iter().enumerate() {
        if !used[idx] {
            problems.push(format!(
                "{}: stale allow entry: ({}, {})",
                c.name, file, first
            ));
        }
    }

    problems
}

fn collect_rs_files(
    dir: &std::path::Path,
    root: &std::path::Path,
    out: &mut Vec<(String, String)>,
) {
    if !dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs_files(&path, root, out);
        } else if path.extension().and_then(|x| x.to_str()) == Some("rs") {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path).unwrap();
            out.push((rel, text));
        }
    }
}

pub fn crate_guarded_files(name: &str) -> Vec<(String, String)> {
    let root = workspace_guards::workspace_root();
    let mut out = Vec::new();
    let cdir = root.join(name);
    for sub in ["src", "tests", "examples", "benches"] {
        collect_rs_files(&cdir.join(sub), &root, &mut out);
    }
    out.sort();
    out
}

#[test]
fn no_lint_suppressions_in_workspace() {
    for c in crate::lint_suppressions_data::TABLE {
        let files = crate_guarded_files(c.name);
        assert!(!files.is_empty(), "{}: scan found no files", c.name);
        let problems = problems_in(c, &files);
        assert!(
            problems.is_empty(),
            "lint suppression found:\n{}",
            problems.join("\n")
        );
    }
    // fuzz/ and tools/ are not workspace members but are still guarded.
    let c = Crate {
        name: "fuzz/tools",
        allow: &[],
    };
    let mut files = Vec::new();
    collect_rs_files(
        &workspace_guards::workspace_root().join("fuzz"),
        &workspace_guards::workspace_root(),
        &mut files,
    );
    collect_rs_files(
        &workspace_guards::workspace_root().join("tools"),
        &workspace_guards::workspace_root(),
        &mut files,
    );
    files.sort();
    assert!(!files.is_empty(), "scan found no fuzz/tools files");
    let problems = problems_in(&c, &files);
    assert!(
        problems.is_empty(),
        "lint suppression found:\n{}",
        problems.join("\n")
    );
}

#[test]
fn table_covers_every_workspace_member() {
    let names: Vec<&str> = crate::lint_suppressions_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    crate::common::assert_names_are_members("lint_suppressions", &names);
    crate::common::assert_table_covers_members("lint_suppressions", &names, &[]);
}

#[cfg(test)]
mod self_tests {
    use super::*;

    #[test]
    fn allow_dead_code_fails() {
        let c = Crate {
            name: "probe",
            allow: &[],
        };
        let files = vec![(
            "src/a.rs".to_string(),
            "#[allow(dead_code)]\nfn f() {}\n".to_string(),
        )];
        let p = problems_in(&c, &files);
        assert!(!p.is_empty(), "{p:?}");
        assert!(p[0].contains("forbidden lint suppression"), "{p:?}");
    }

    #[test]
    fn expect_clippy_fails() {
        let c = Crate {
            name: "probe",
            allow: &[],
        };
        let files = vec![(
            "src/a.rs".to_string(),
            "#[expect(clippy::too_many_arguments)]\nfn f() {}\n".to_string(),
        )];
        let p = problems_in(&c, &files);
        assert!(!p.is_empty(), "{p:?}");
    }

    #[test]
    fn attribute_in_comment_or_string_passes() {
        let c = Crate {
            name: "probe",
            allow: &[],
        };
        let files = vec![(
            "src/a.rs".to_string(),
            "// #[allow(dead_code)]\nfn f() { let s = \"#[expect(clippy::x)]\"; }\n".to_string(),
        )];
        let p = problems_in(&c, &files);
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn unlisted_unsafe_code_fails() {
        let c = Crate {
            name: "probe",
            allow: &[],
        };
        let files = vec![(
            "src/a.rs".to_string(),
            "#[allow(unsafe_code)]\nmod m {\n    // SAFETY: x\n}\n".to_string(),
        )];
        let p = problems_in(&c, &files);
        assert!(!p.is_empty(), "{p:?}");
    }

    #[test]
    fn listed_unsafe_code_without_safety_fails() {
        let c = Crate {
            name: "probe",
            allow: &[("src/a.rs", "mod m {", "reason")],
        };
        let files = vec![(
            "src/a.rs".to_string(),
            "#[allow(unsafe_code)]\nmod m {\n}\n".to_string(),
        )];
        let p = problems_in(&c, &files);
        assert!(!p.is_empty(), "{p:?}");
        assert!(p.iter().any(|x| x.contains("without SAFETY")), "{p:?}");
    }

    #[test]
    fn listed_unsafe_code_with_safety_passes() {
        let c = Crate {
            name: "probe",
            allow: &[("src/a.rs", "mod m {", "reason")],
        };
        let files = vec![(
            "src/a.rs".to_string(),
            "#[allow(unsafe_code)]\nmod m {\n    // SAFETY: ok\n}\n".to_string(),
        )];
        let p = problems_in(&c, &files);
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn used_static_fails() {
        let c = Crate {
            name: "probe",
            allow: &[],
        };
        let files = vec![(
            "src/a.rs".to_string(),
            "#[used]\nstatic X: () = { foo(); };\n".to_string(),
        )];
        let p = problems_in(&c, &files);
        assert!(!p.is_empty(), "{p:?}");
        assert!(p[0].contains("forbidden lint suppression"), "{p:?}");
    }

    #[test]
    fn let_underscore_in_const_block_fails() {
        let c = Crate {
            name: "probe",
            allow: &[],
        };
        let files = vec![(
            "src/a.rs".to_string(),
            "const _: () = {\n    let _ = foo;\n};\n".to_string(),
        )];
        let p = problems_in(&c, &files);
        assert!(!p.is_empty(), "{p:?}");
    }

    #[test]
    fn scanner_bites() {
        let src = "#[allow(dead_code)]\nfn f() {}\n";
        assert_eq!(hits_in(src).len(), 1);
        let src = "// #[allow(dead_code)]\nfn f() {}\n";
        assert!(hits_in(src).is_empty());
    }
}
