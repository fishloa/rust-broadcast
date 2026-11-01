//! Convention audit (acceptance gate, see `docs/DESCRIPTOR-ACCEPTANCE.md`).
//!
//! Mechanically enforces, for every descriptor module (any file that
//! `impl`s `DescriptorDef` or `ExtensionBodyDef`), the conventions CI does not
//! otherwise check:
//!   1. the module doc cites a spec (standard + section/tag), and
//!   2. an in-module round-trip test exists.
//!
//! These are the project's defining discipline (spec grounding + symmetric
//! round-trip) and were previously review-only. New descriptors that forget
//! either now fail the gate.

use std::fs;
use std::path::{Path, PathBuf};

/// Collect `(path, contents)` for every `.rs` under `dir`.
fn read_rs(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            read_rs(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            let body = fs::read_to_string(&path).expect("read .rs");
            out.push((path, body));
        }
    }
}

/// The leading `//!` module-doc block of a source file.
fn module_doc(body: &str) -> String {
    body.lines()
        .take_while(|l| {
            let t = l.trim_start();
            t.starts_with("//!") || t.is_empty() || t.starts_with("//")
        })
        .filter(|l| l.trim_start().starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Does the module doc cite a spec (standard id, section, or tag)?
fn cites_spec(doc: &str) -> bool {
    const MARKERS: &[&str] = &[
        "§",
        "Table ",
        "EN 300",
        "EN 302",
        "EN 303",
        "ISO/IEC",
        "H.222",
        "TS 102",
        "TS 101",
        "TS 103",
        "TR 101",
        "NorDig",
        "tag_extension",
        "(tag 0x",
        "Annex",
    ];
    MARKERS.iter().any(|m| doc.contains(m))
}

#[test]
fn every_descriptor_module_cites_spec_and_round_trips() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/descriptors");
    let mut files = Vec::new();
    read_rs(&root, &mut files);

    let mut problems: Vec<String> = Vec::new();
    let mut audited = 0usize;

    // Infra files reference the `*Def` traits (macro / blanket registration
    // impls) but are not per-descriptor modules.
    const SKIP: &[&str] = &["any.rs", "mod.rs", "registry.rs", "test_support.rs"];

    for (path, body) in &files {
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if SKIP.contains(&fname) {
            continue;
        }
        let is_descriptor = body.lines().any(|l| {
            l.contains("impl")
                && l.contains(" for ")
                && (l.contains("DescriptorDef") || l.contains("ExtensionBodyDef"))
        });
        if !is_descriptor {
            continue;
        }
        audited += 1;
        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(path)
            .display();

        if !cites_spec(&module_doc(body)) {
            problems.push(format!(
                "{rel}: module doc has no spec citation (§/Table/standard id)"
            ));
        }
        // In-module round-trip test (the convention names them `*round_trip*`).
        if !body.contains("round_trip") {
            problems.push(format!(
                "{rel}: no in-module round-trip test (`round_trip`)"
            ));
        }
    }

    assert!(
        audited > 50,
        "audit found only {audited} descriptor modules — walk broken?"
    );
    assert!(
        problems.is_empty(),
        "convention audit failed ({} issue(s)) — see docs/DESCRIPTOR-ACCEPTANCE.md:\n{}",
        problems.len(),
        problems.join("\n")
    );
}
