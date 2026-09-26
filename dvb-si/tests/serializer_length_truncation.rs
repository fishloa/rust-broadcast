//! Regression guard for the serializer length/count truncation class (#1129).
//!
//! `descriptors/network_name.rs` used to write `descriptor_length` with
//! `buf[1] = self.network_name.len() as u8;` — no range check, so a body
//! longer than 255 bytes silently wrapped and misframed the rest of the
//! descriptor loop. The same shape (`<expr>.len() as u8/u16/u32`, applied
//! directly to a length/count with no prior range check) recurred across
//! ~40 descriptor serializers.
//!
//! The fix is `broadcast_common::len::fit_u8`/`fit_u16`/`fit_bits`, which
//! compare before narrowing and return `Err` instead of wrapping. This test
//! is the cheap, ungameable half of the regression guard: it fails CI if the
//! literal truncating pattern reappears anywhere under `src/descriptors/`
//! outside a `#[cfg(test)]` module (test fixtures are allowed to poke raw
//! bytes together by hand). See also the per-descriptor boundary tests
//! (network_name, telephone, audio_preselection, target_region_name,
//! vvc_subpictures, protection_message, data_broadcast, multilingual_*, …)
//! that prove `serialize_into` actually returns `Err` for an over-range
//! field, which this source scan cannot show by itself.

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

/// The truncating pattern: a length/count narrowed straight off `.len()`
/// with no intervening range check. Deliberately does NOT match
/// `x.len() as u64` (widening, harmless) or `fit_u8(x.len(), ..)? as u8`
/// (checked first, then narrowed) — only the direct, unchecked narrow.
fn has_truncating_cast(line: &str) -> bool {
    let Some(idx) = line.find(".len()") else {
        return false;
    };
    let rest = line[idx + ".len()".len()..].trim_start();
    for width in ["as u8", "as u16", "as u32"] {
        if let Some(r) = rest.strip_prefix(width) {
            // Must be a cast boundary, not e.g. `as u8x16` or similar.
            if r.chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_')
            {
                return true;
            }
        }
    }
    false
}

/// Byte offset of the first `#[cfg(test)]` in the file, if any. Everything
/// from there on is treated as test fixture code, which is allowed to build
/// raw wire bytes by hand (that's the whole point of a fixture).
fn test_module_start(body: &str) -> Option<usize> {
    body.find("#[cfg(test)]")
}

#[test]
fn no_unchecked_len_narrowing_in_descriptor_serializers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/descriptors");
    let mut files = Vec::new();
    read_rs(&root, &mut files);

    let mut problems: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    for (path, body) in &files {
        scanned += 1;
        let production_end = test_module_start(body).unwrap_or(body.len());
        let production = &body[..production_end];

        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(path)
            .display();

        for (i, line) in production.lines().enumerate() {
            if has_truncating_cast(line) {
                problems.push(format!(
                    "{rel}:{}: unchecked `.len() as uN` — use broadcast_common::len::fit_u8/fit_u16/fit_bits instead (#1129): {}",
                    i + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        scanned > 100,
        "scan found only {scanned} files under src/descriptors — walk broken?"
    );
    assert!(
        problems.is_empty(),
        "found {} unchecked length-narrowing site(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
