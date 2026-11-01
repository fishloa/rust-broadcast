//! Lexical tripwire (audit S3): no panicking idiom in non-test code of
//! `dvb-subtitle`. Library parse/serialize/convert paths must return a typed
//! error for hostile input, never `.unwrap()` / `.expect(..)` / `panic!` /
//! `unreachable!` / `todo!` / `unimplemented!`. `#[cfg(test)]` modules are
//! exempt; a justified exception is added to `ALLOW` with a reason, pinned to
//! the exact source text. A tripwire is lexical, not a proof (an index
//! `buf[i]` can still panic): the behavioural half is `tests/it/hostility.rs`.

use std::fs;
use std::path::{Path, PathBuf};

/// `(file suffix, line substring, reason)`.
const ALLOW: &[(&str, &str, &str)] = &[];

const FORBIDDEN: &[&str] = &[
    ".unwrap()",
    ".expect(",
    "panic!(",
    "unreachable!(",
    "todo!(",
    "unimplemented!(",
];

fn read_rs(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            read_rs(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push((path.clone(), fs::read_to_string(&path).expect("read .rs")));
        }
    }
}

/// The non-test prefix of a file: everything before the first
/// `#[cfg(test)]` / `#[cfg(all(test, ..))]`.
fn non_test(body: &str) -> &str {
    let cut = ["#[cfg(test)]", "#[cfg(all(test"]
        .iter()
        .filter_map(|m| body.find(m))
        .min()
        .unwrap_or(body.len());
    &body[..cut]
}

fn hits_in(body: &str) -> Vec<(usize, String)> {
    non_test(body)
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            let t = l.trim_start();
            !t.starts_with("//") && FORBIDDEN.iter().any(|f| l.contains(f))
        })
        .map(|(n, l)| (n + 1, l.trim().to_string()))
        .collect()
}

#[test]
fn no_panicking_idiom_in_non_test_code() {
    let mut files = Vec::new();
    read_rs(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(!files.is_empty(), "scan found no files under src");
    let mut problems = Vec::new();
    for (path, body) in &files {
        for (n, line) in hits_in(body) {
            let p = path.to_string_lossy();
            if ALLOW
                .iter()
                .any(|(f, sub, _)| p.ends_with(f) && line.contains(sub))
            {
                continue;
            }
            problems.push(format!("{p}:{n}: {line}"));
        }
    }
    assert!(
        problems.is_empty(),
        "panicking idiom in non-test code (return a typed error, or allow-list with a reason):\n{}",
        problems.join("\n")
    );
}

/// The scanner must bite.
#[test]
fn scanner_bites() {
    assert_eq!(hits_in("fn f() {\n    x.unwrap();\n}\n").len(), 1);
    assert_eq!(hits_in("fn f() {\n    x.expect(\"m\");\n}\n").len(), 1);
    assert_eq!(hits_in("fn f() {\n    unreachable!();\n}\n").len(), 1);
    // test modules and comments are exempt
    assert!(hits_in("fn f() {}\n#[cfg(test)]\nmod t { fn g() { x.unwrap(); } }\n").is_empty());
    assert!(hits_in("// x.unwrap()\nfn f() {}\n").is_empty());
}
