//! Serializer length/count truncation guard for `dvb-ci` (#1129).
//!
//! A wire length/count must be range-checked before it is narrowed:
//! `buf[0] = x.len() as u8;` silently wraps a 256-byte body to length 0 and
//! misframes everything after it. The fix is
//! `broadcast_common::len::fit_u8`/`fit_u16`/`fit_u32`/`fit_bits` (compare in
//! `usize`, then narrow) or an explicit `> MAX` early return.
//!
//! This is the cheap lexical half of the guard (the behavioural half is each
//! type's own over-range test): no production line (outside `#[cfg(test)]`)
//! may narrow `<expr>.len()` / `<expr>.count()` with `as u8|u16|u32` unless
//! its enclosing function also contains a range check, or the exact line is on
//! `ALLOWLIST` with a reason. A stale allowlist entry fails the guard so the
//! list cannot rot.

use std::path::{Path, PathBuf};

/// `(file, exact trimmed line, reason)`.
const ALLOWLIST: &[(&str, &str, &str)] = &[
    (
        "src/ci_plus/cicam_player.rs",
        "buf[0..2].copy_from_slice(&(loc.len() as u16).to_be_bytes());",
        "write_apdu_header runs first and length::encode_with rejects any APDU body > 0xFFFF (LengthTooLarge), so loc.len() + 2 <= 0xFFFF.",
    ),
    (
        "src/ci_plus/cicam_player.rs",
        "buf[pos..pos + 2].copy_from_slice(&(self.component_types.len() as u16).to_be_bytes());",
        "write_apdu_header rejects body > 0xFFFF first; count * 2 + 2 <= 0xFFFF.",
    ),
    (
        "src/ci_plus/cicam_player.rs",
        "buf[pos + 5..pos + 7].copy_from_slice(&(self.pmt.len() as u16).to_be_bytes());",
        "write_apdu_header rejects body > 0xFFFF first; pmt.len() + 7 <= 0xFFFF.",
    ),
    (
        "src/ci_plus/cicam_player.rs",
        "buf[pos + 1..pos + 3].copy_from_slice(&(self.pmt.len() as u16).to_be_bytes());",
        "write_apdu_header rejects body > 0xFFFF first; pmt.len() + 3 <= 0xFFFF.",
    ),
    (
        "src/builder.rs",
        "out.push(body.len() as u8);",
        "body comes from DescriptorLoop::raw_tags, which already validated it fits the declared 8-bit descriptor_length.",
    ),
    (
        "src/ci_plus/low_speed_comms_v4.rs",
        "buf[p + 3] = self.source_addresses.len() as u8;",
        "bounded by write_descriptor_header just above, which rejects descriptor_length > 255 (count * 17 + fixed <= 255).",
    ),
    (
        "src/ci_plus/sample_decryption.rs",
        "w.u16(self.drm_metadata.len() as u16);",
        "validate() (range-checks drm_metadata_length, 16 bits) always runs before write_into.",
    ),
    (
        "src/ci_plus/sample_decryption.rs",
        "w.u8(records.len() as u8);",
        "validate() range-checks number_of_metadata_records (8 bits) before write_into.",
    ),
    (
        "src/ci_plus/sample_decryption.rs",
        "w.u8(tracks.len() as u8);",
        "validate() range-checks number_of_Sample_Tracks (8 bits) before write_into.",
    ),
    (
        "src/ci_plus/sample_decryption.rs",
        "w.u8(track.records.len() as u8);",
        "validate() range-checks the per-track number_of_metadata_records (8 bits) before write_into.",
    ),
];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read dir") {
        let p = e.expect("dir entry").path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `.len()` / `.count()` immediately narrowed with `as u8|u16|u32`.
fn narrows_len(line: &str) -> bool {
    let mut rest = line;
    while let Some(i) = rest.find(')') {
        let (head, tail) = rest.split_at(i + 1);
        rest = tail;
        if head.ends_with(".len()") || head.ends_with(".count()") {
            let t = tail.trim_start();
            for w in ["as u8", "as u16", "as u32"] {
                if t.strip_prefix(w).is_some_and(|r| {
                    r.chars()
                        .next()
                        .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                }) {
                    return true;
                }
            }
        }
    }
    false
}

/// Does the function text contain a range check?
fn has_guard(text: &str) -> bool {
    ["fit_", "FieldOverflow", "try_from", "::MAX", "_MAX", "MAX_"]
        .iter()
        .any(|t| text.contains(t))
        || text.contains("> 0x")
        || text.lines().any(|l| {
            let l = l.trim_start();
            (l.starts_with("if ") || l.contains("&& ")) && (l.contains(" > ") || l.contains(" >= "))
        })
}

#[test]
fn len_narrowing_sites_are_range_checked_or_allowlisted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rs_files(&root.join("src"), &mut files);
    assert!(!files.is_empty(), "scan found no source files");

    let mut problems = Vec::new();
    let mut used = vec![false; ALLOWLIST.len()];
    for path in &files {
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let body = std::fs::read_to_string(path).unwrap();
        let prod = &body[..body.find("#[cfg(test)]").unwrap_or(body.len())];
        let lines: Vec<&str> = prod.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let t = line.trim();
            if t.starts_with("//") || !narrows_len(t) {
                continue;
            }
            if let Some(k) = ALLOWLIST.iter().position(|(f, l, _)| *f == rel && *l == t) {
                used[k] = true;
                continue;
            }
            let mut start = n;
            while start > 0 && !lines[start].contains("fn ") {
                start -= 1;
            }
            if !has_guard(&lines[start..=n].join("\n")) {
                problems.push(format!(
                    "{rel}:{}: unchecked length narrowing (use broadcast_common::len::fit_*): {t}",
                    n + 1
                ));
            }
        }
    }
    for (k, u) in used.iter().enumerate() {
        assert!(
            *u,
            "stale allowlist entry: {} :: {}",
            ALLOWLIST[k].0, ALLOWLIST[k].1
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn guard_detector_bites() {
    assert!(narrows_len("buf[0] = x.len() as u8;"));
    assert!(narrows_len("f(&(a.b.count() as u32));"));
    assert!(!narrows_len("let n = x.len() as u64;"));
    assert!(!narrows_len("let n = x.len() as usize;"));
    assert!(!has_guard("fn f() { buf[0] = x.len() as u8; }"));
    assert!(has_guard("fn f() { let n = fit_u8(x.len(), \"a\")?; }"));
    assert!(has_guard("fn f() {\n if x.len() > 255 { return; }\n }"));
}
