//! EBML element-ID literals live in exactly one crate (streaming-first Phase 5).
//!
//! Every EBML/Matroska Element ID, the VINT size encoding and the element walk
//! are `broadcast-matroska`'s. A workspace member that spells a Matroska
//! element ID itself is drift: the two copies `transmux` carries today
//! (`webm_demux.rs` reading, `mkv_mux.rs` writing) disagree with each other and
//! with RFC 9559, and they are the reason this crate exists.
//!
//! The scan looks for the distinctive four-octet IDs, spelled either as a
//! `u32` hexadecimal literal (underscores optional) or as a `[u8; 4]` byte
//! array. One-octet IDs (`0xAE`, `0x86`) are far too common to police lexically.
//!
//! **This is a lexical tripwire, not a proof.** An ID assembled at runtime, a
//! renamed helper or a macro can evade it; code review is the real control.
//! Every allowlist entry is `(crate-relative path, exact trimmed line, reason)`
//! and a stale entry (no longer matching a banned line) fails.
//!
//! The one long-lived exception is `container-probe`: it must depend only on
//! `broadcast-common` at runtime, so it keeps its magic and is pinned to this
//! crate by dev-only drift tests. `transmux`'s entries are the hand-written
//! WebM/Matroska code this crate replaces in Phase 5 slices 3–4.

use workspace_guards::src_files;

pub struct Crate {
    pub name: &'static str,
    /// `(path suffix, exact trimmed line, reason)` — reviewed, line-pinned
    /// exceptions.
    pub allow: &'static [(&'static str, &'static str, &'static str)],
}

/// The four-octet Matroska element IDs no crate outside `broadcast-matroska`
/// may spell itself (RFC 9559 §5).
pub const EBML_IDS: &[[u8; 4]] = &[
    [0x1A, 0x45, 0xDF, 0xA3], // EBML
    [0x18, 0x53, 0x80, 0x67], // Segment
    [0x11, 0x4D, 0x9B, 0x74], // SeekHead
    [0x15, 0x49, 0xA9, 0x66], // Info
    [0x16, 0x54, 0xAE, 0x6B], // Tracks
    [0x1F, 0x43, 0xB6, 0x75], // Cluster
    [0x1C, 0x53, 0xBB, 0x6B], // Cues
    [0x12, 0x54, 0xC3, 0x67], // Tags
    [0x10, 0x43, 0xA7, 0x70], // Chapters
    [0x19, 0x41, 0xA4, 0x69], // Attachments
];

/// The `u32` spelling with underscores removed, e.g. `0x1a45dfa3`.
fn int_needle(id: &[u8; 4]) -> String {
    let mut out = String::from("0x");
    for byte in id {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The byte-array spelling with whitespace, case and separators normalised,
/// e.g. `[0x1a,0x45,0xdf,0xa3]`.
fn array_needle(id: &[u8; 4]) -> String {
    let parts: Vec<String> = id.iter().map(|byte| format!("0x{byte:02x}")).collect();
    format!("[{}]", parts.join(","))
}

fn compact_lower(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn check(c: &Crate, files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut used = vec![false; c.allow.len()];
    for (rel, text) in files {
        let source = workspace_guards::scan::non_test_source_blanked(text);
        let masked = workspace_guards::scan::blank_comments_and_strings(&source);
        for (index, line) in masked.lines().enumerate() {
            let compact = compact_lower(line);
            let Some(needle) = EBML_IDS.iter().find(|id| {
                compact.contains(&int_needle(id)) || compact.contains(&array_needle(id))
            }) else {
                continue;
            };
            let original = text.lines().nth(index).unwrap_or("");
            let trimmed = original.trim();
            if let Some(slot) = c
                .allow
                .iter()
                .position(|(suffix, allowed, _)| rel.ends_with(suffix) && *allowed == trimmed)
            {
                used[slot] = true;
            } else {
                violations.push(format!(
                    "{}: {rel}:{}: EBML element ID {:02X?} spelled outside broadcast-matroska: {trimmed}",
                    c.name,
                    index + 1,
                    needle
                ));
            }
        }
    }
    for (a, u) in c.allow.iter().zip(&used) {
        if !*u {
            violations.push(format!(
                "{}: stale ALLOW entry: ({}, {:?})",
                c.name, a.0, a.1
            ));
        }
    }
    violations
}

#[test]
fn no_ebml_element_ids_outside_broadcast_matroska() {
    let mut violations = Vec::new();
    for member in workspace_guards::workspace_members() {
        if member == "broadcast-matroska"
            || crate::one_ebml_home_data::EXEMPT.contains(&member.as_str())
        {
            continue;
        }
        let allow = crate::one_ebml_home_data::ALLOW
            .iter()
            .find(|c| c.name == member.as_str())
            .map_or(&[][..], |c| c.allow);
        let files = src_files(&member);
        assert!(!files.is_empty(), "{member}: no source files under src/");
        let guard = Crate {
            name: "one_ebml_home",
            allow,
        };
        violations.extend(check(&guard, &files));
    }
    assert!(
        violations.is_empty(),
        "EBML element-ID syntax outside `broadcast-matroska` (use its ids and element model, or add a reasoned line-pinned ALLOW entry in one_ebml_home_data.rs):\n{}",
        violations.join("\n")
    );
}

#[test]
fn allowlist_names_are_workspace_members() {
    let names: Vec<&str> = crate::one_ebml_home_data::ALLOW
        .iter()
        .map(|c| c.name)
        .collect();
    crate::common::assert_names_are_members("one_ebml_home", &names);
}

#[test]
fn detector_bites() {
    let c = Crate {
        name: "synthetic",
        allow: &[("src/ok.rs", "const SEGMENT: u32 = 0x1853_8067;", "test")],
    };
    let files = vec![
        (
            "src/a.rs".to_string(),
            "const SEGMENT: u32 = 0x1853_8067;\n".to_string(),
        ),
        (
            "src/b.rs".to_string(),
            "const ID_EBML: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];\n".to_string(),
        ),
        (
            "src/c.rs".to_string(),
            "// 0x18538067 in a comment is fine\n".to_string(),
        ),
        (
            "src/d.rs".to_string(),
            "#[cfg(test)]\nmod tests {\n    const SEGMENT: u32 = 0x1853_8067;\n}\n".to_string(),
        ),
        (
            "src/ok.rs".to_string(),
            "const SEGMENT: u32 = 0x1853_8067;\n".to_string(),
        ),
    ];
    let violations = check(&c, &files);
    assert_eq!(violations.len(), 2, "{violations:#?}");
    assert!(violations.iter().any(|v| v.contains("a.rs:1")));
    assert!(violations.iter().any(|v| v.contains("b.rs:1")));
}
