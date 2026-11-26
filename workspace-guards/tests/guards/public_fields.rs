//! Tripwire: a public struct in the listed crates keeps its fields private.
//!
//! Every public struct in `broadcast-codec` exposes a constructor or builder and
//! accessor methods, never a public field: a field is added, renamed or retyped
//! as a reviewable API change instead of a silent struct-literal break at every
//! call site.
//!
//! **This is a lexical tripwire, not a proof.** It detects a `pub` field only in
//! the named-field form and the one-line tuple form, with the struct header and
//! its opening brace on the same line; a field written through a macro or split
//! across lines can evade it, so code review is the real control.

use workspace_guards::src_files;

/// Crates whose public structs must have private fields.
pub const CRATES: &[&str] = &["broadcast-codec"];

/// Whether a struct-body line begins a field with an unrestricted `pub`.
fn is_pub_field(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("pub ") else {
        return false;
    };
    if rest.starts_with("fn ")
        || rest.starts_with("const ")
        || rest.starts_with("type ")
        || rest.starts_with("struct ")
        || rest.starts_with("enum ")
        || rest.starts_with("trait ")
        || rest.starts_with("use ")
        || rest.starts_with("mod ")
    {
        return false;
    }
    rest.contains(':')
}

/// Findings as `"crate: file:line: …"`, with the offending line quoted.
fn scan(name: &str, files: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, body) in files {
        let production = workspace_guards::scan::non_test_source_blanked(body);
        let mut struct_name: Option<String> = None;
        let mut depth: usize = 0;
        for (number, line) in production.lines().enumerate() {
            let trimmed = line.trim();
            let mut rest = trimmed;
            loop {
                if struct_name.is_none() {
                    let Some(start) = rest.find("pub struct ") else {
                        break;
                    };
                    let after = &rest[start + "pub struct ".len()..];
                    let type_name = after
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect::<String>();
                    let open_tuple = after.find('(');
                    let open_brace = after.find('{');
                    if let Some(tuple) = open_tuple
                        && open_brace.is_none_or(|brace| tuple < brace)
                    {
                        let tuple_body = &after[tuple..];
                        if tuple_body.contains("pub ") {
                            problems.push(format!(
                                "{name}: {path}:{}: public tuple field in `{type_name}`: {trimmed}",
                                number + 1
                            ));
                        }
                        break;
                    }
                    let Some(brace) = open_brace else {
                        break;
                    };
                    struct_name = Some(type_name);
                    depth = 1;
                    rest = &after[brace + 1..];
                    continue;
                }
                if is_pub_field(rest.trim_start()) {
                    let type_name = struct_name.as_deref().unwrap_or_default();
                    problems.push(format!(
                        "{name}: {path}:{}: public field in `{type_name}`: {trimmed}",
                        number + 1
                    ));
                }
                depth += rest.matches('{').count();
                depth = depth.saturating_sub(rest.matches('}').count());
                if depth == 0 {
                    struct_name = None;
                }
                break;
            }
        }
    }
    problems
}

#[test]
fn public_struct_fields_are_private() {
    for name in CRATES {
        let problems = scan(name, &src_files(name));
        assert!(
            problems.is_empty(),
            "public struct fields must be private (use a constructor/builder and accessors):\n{}",
            problems.join("\n")
        );
    }
}

#[test]
fn table_names_are_workspace_members() {
    let members = workspace_guards::workspace_members();
    for name in CRATES {
        assert!(
            members.iter().any(|m| m == name),
            "public_fields: `{name}` is not a workspace member"
        );
    }
}

#[test]
fn detector_bites() {
    let files = vec![(
        "src/a.rs".to_string(),
        "pub struct Good { hidden: u8 }\n\
         pub struct Bad { pub field: u8 }\n\
         pub struct Tuple(pub u8);\n\
         pub struct WithMethod { hidden: u8 }\n\
         impl WithMethod { pub fn value(&self) -> u8 { self.hidden } }\n"
            .to_string(),
    )];
    let found = scan("synthetic", &files);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].contains("public field in `Bad`"), "{found:?}");
    assert!(
        found[1].contains("public tuple field in `Tuple`"),
        "{found:?}"
    );
}
