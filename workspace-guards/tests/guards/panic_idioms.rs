//! Lexical tripwire (audit S3): no panicking idiom in non-test code of a table
//! crate. Library parse/serialize/convert paths must return a typed error for
//! hostile input, never `.unwrap()` / `.expect(..)` / `panic!` /
//! `unreachable!` / `todo!` / `unimplemented!`. `#[cfg(test)]` modules are
//! exempt; a justified exception is added to the crate's `allow` with a
//! reason, pinned to the exact source text. A tripwire is lexical, not a proof
//! (an index `buf[i]` can still panic): the behavioural half is each crate's
//! `hostility` test.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// `(file suffix, line substring, reason)`.
    pub allow: &'static [(&'static str, &'static str, &'static str)],
}

const FORBIDDEN: &[&str] = &[
    ".unwrap()",
    ".expect(",
    "panic!(",
    "unreachable!(",
    "todo!(",
    "unimplemented!(",
];

/// The source without its test code: every item carrying `#[cfg(test)]` or
/// `#[cfg(all(test, ..))]`, blanked so line numbers still match the file.
fn non_test(body: &str) -> String {
    workspace_guards::scan::without_attributed_items(body, |attr| {
        attr.starts_with("#[cfg(test)]") || attr.starts_with("#[cfg(all(test")
    })
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

fn problems_in(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, body) in files {
        for (n, line) in hits_in(body) {
            if c.allow
                .iter()
                .any(|(f, sub, _)| path.ends_with(f) && line.contains(sub))
            {
                continue;
            }
            problems.push(format!("{}: {path}:{n}: {line}", c.name));
        }
    }
    problems
}

#[test]
fn no_panicking_idiom_in_non_test_code() {
    let mut problems = Vec::new();
    for c in crate::panic_idioms_data::TABLE {
        let files = src_files(c.name);
        assert!(
            !files.is_empty(),
            "{}: scan found no files under src",
            c.name
        );
        problems.extend(problems_in(c, &files));
    }
    assert!(
        problems.is_empty(),
        "panicking idiom in non-test code (return a typed error, or allow-list with a reason):\n{}",
        problems.join("\n")
    );
}

#[test]
fn table_names_are_workspace_members() {
    let names: Vec<&str> = crate::panic_idioms_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    crate::common::assert_names_are_members("panic_idioms", &names);
}

/// The scanner must bite.
#[test]
fn scanner_bites() {
    assert_eq!(hits_in("fn f() {\n    x.unwrap();\n}\n").len(), 1);
    assert_eq!(hits_in("fn f() {\n    x.expect(\"m\");\n}\n").len(), 1);
    assert_eq!(hits_in("fn f() {\n    unreachable!();\n}\n").len(), 1);
    assert!(hits_in("fn f() {}\n#[cfg(test)]\nmod t { fn g() { x.unwrap(); } }\n").is_empty());
    assert!(hits_in("// x.unwrap()\nfn f() {}\n").is_empty());
}

#[test]
fn allow_entry_pins_file_and_text() {
    let c = Crate {
        name: "probe",
        allow: &[("a.rs", "x.unwrap()", "reason")],
    };
    let files = vec![
        (
            "src/a.rs".to_string(),
            "fn f() {\n    x.unwrap();\n}\n".to_string(),
        ),
        (
            "src/b.rs".to_string(),
            "fn f() {\n    x.unwrap();\n}\n".to_string(),
        ),
    ];
    let problems = problems_in(&c, &files);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("src/b.rs"));
}

#[test]
fn a_test_module_in_the_middle_does_not_hide_the_code_after_it() {
    let body = "#[cfg(test)]\nmod t {\n    fn f() {}\n}\n#[ignore]\nfn g() { x.unwrap(); }\n";
    let hits = hits_in(body);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].0, 6, "the line number is the one in the file");
}
