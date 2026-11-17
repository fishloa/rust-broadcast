//! Lexical tripwire: no process-default rustls builder in non-test source of a
//! crate that depends on the rustls stack. `ClientConfig::builder()` and
//! `ServerConfig::builder()` take the process-default `CryptoProvider`, which
//! panics whenever more than one provider is linked; the workspace standard is
//! ring, selected explicitly with `builder_with_provider`. Test code is exempt.
//! A tripwire is lexical, not a proof: a call split across lines, or reached
//! through a wrapper, can evade it.

use workspace_guards::scan::non_test_source_blanked;
use workspace_guards::{crate_dir, src_files};

pub struct Crate {
    pub name: &'static str,
}

const BANNED: [&str; 2] = ["ClientConfig::builder()", "ServerConfig::builder()"];

const RUSTLS_STACK: [&str; 4] = ["rustls", "tokio-rustls", "hyper-rustls", "reqwest"];

fn problems_in(name: &str, files: &[(String, String)]) -> Vec<String> {
    let mut hits = Vec::new();
    for (path, text) in files {
        for (no, line) in non_test_source_blanked(text).lines().enumerate() {
            if let Some(call) = BANNED.iter().find(|b| line.contains(*b)) {
                hits.push(format!(
                    "{name}: {path}:{}: `{call}` takes the process-default CryptoProvider; use `builder_with_provider`",
                    no + 1
                ));
            }
        }
    }
    hits
}

/// The dependency key a manifest line declares (`rustls = ...`, or the
/// `[dependencies.rustls]` table form), if it is a rustls-stack dependency.
fn declared_stack_dep(line: &str) -> Option<&'static str> {
    let t = line.trim_start();
    if t.starts_with('#') {
        return None;
    }
    if let Some(rest) = t.strip_prefix('[') {
        let table = rest.trim_end_matches(']');
        let key = table.rsplit_once("dependencies.")?.1;
        return RUSTLS_STACK.iter().copied().find(|k| *k == key);
    }
    let key: String = t
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    RUSTLS_STACK.iter().copied().find(|k| *k == key)
}

fn manifest_declares_stack(name: &str) -> bool {
    let manifest = crate_dir(name).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
    text.lines().any(|l| declared_stack_dep(l).is_some())
}

#[test]
fn src_has_no_process_default_rustls_builder() {
    let mut hits = Vec::new();
    for c in crate::tls_provider_data::TABLE {
        let files = src_files(c.name);
        assert!(!files.is_empty(), "{}: no files under src", c.name);
        hits.extend(problems_in(c.name, &files));
    }
    assert!(
        hits.is_empty(),
        "TLS provider guard violation(s): {hits:#?}"
    );
}

#[test]
fn table_names_are_workspace_members() {
    let names: Vec<&str> = crate::tls_provider_data::TABLE
        .iter()
        .map(|c| c.name)
        .collect();
    crate::common::assert_names_are_members("tls_provider", &names);
}

#[test]
fn table_covers_every_rustls_dependent() {
    let members = workspace_guards::workspace_members();
    let mut problems = Vec::new();
    for m in &members {
        if m == "workspace-guards" {
            continue;
        }
        let listed = crate::tls_provider_data::TABLE.iter().any(|c| c.name == m);
        let declares = manifest_declares_stack(m);
        if declares && !listed {
            problems.push(format!(
                "tls_provider: `{m}` declares a rustls-stack dependency but is not in the table"
            ));
        }
        if listed && !declares {
            problems.push(format!(
                "tls_provider: `{m}` is in the table but declares no rustls-stack dependency"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn detector_flags_process_default_builders_and_allows_explicit_provider() {
    let files = vec![(
        "src/a.rs".to_string(),
        "fn a() {\n    let c = rustls::ClientConfig::builder().with_no_client_auth();\n    let s = rustls::ServerConfig::builder_with_provider(p()).with_no_client_auth();\n    let t = rustls::ServerConfig::builder();\n}\n"
            .to_string(),
    )];
    let hits = problems_in("probe", &files).join("\n");
    assert!(hits.contains("src/a.rs:2"), "{hits}");
    assert!(!hits.contains("src/a.rs:3"), "{hits}");
    assert!(hits.contains("src/a.rs:4"), "{hits}");
}

#[test]
fn detector_ignores_test_only_code() {
    let files = vec![(
        "src/a.rs".to_string(),
        "#[cfg(test)]\nmod tests {\n    fn t() {\n        let _ = rustls::ClientConfig::builder();\n    }\n}\n"
            .to_string(),
    )];
    assert!(problems_in("probe", &files).is_empty());
}

#[test]
fn declared_stack_dep_reads_inline_and_table_forms() {
    assert_eq!(
        declared_stack_dep("rustls = { version = \"0.23\" }"),
        Some("rustls")
    );
    assert_eq!(
        declared_stack_dep("tokio-rustls  = { version = \"0.26\" }"),
        Some("tokio-rustls")
    );
    assert_eq!(
        declared_stack_dep("[dependencies.reqwest]"),
        Some("reqwest")
    );
    assert_eq!(declared_stack_dep("# rustls = \"0.23\""), None);
    assert_eq!(declared_stack_dep("rustls-pki-types = \"1\""), None);
}
