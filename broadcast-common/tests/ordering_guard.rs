//! Ordering tripwire (story O1, issue #1237): the big registry tables must
//! stay in ascending wire-value order, and every table that mirrors another
//! (enum variants, `from_*`/`to_*` arms, `name()` arms, `declare_*!` lists)
//! must keep the same order. Inserting a new entry "next to the related one"
//! instead of at its numeric position (the `StreamType` 0xD2..0xD5 AVS rows
//! that landed before 0x81/0x86/0x87, the `ExtensionTag` DTS rows after 0x13)
//! is exactly the regression this guards.
//!
//! **Lexical, not a proof**: it scans source text for the anchors below.
//! Scope (file -> what is checked):
//!
//! - `dvb-si/src/descriptor_tag.rs`, `table_id.rs`, `descriptors/extension/mod.rs`:
//!   enum discriminants strictly ascending.
//! - `dvb-si/src/tables/pmt.rs` `StreamType`: `from_u8` and `to_u8` arms ascending,
//!   enum variants / `to_u8` / `name()` arms in the same order, `/// 0xNN` doc rows ascending.
//! - `dvb-si/src/descriptors/service.rs` `ServiceType`, `linkage.rs` `LinkageType`:
//!   `from_u8`/`to_u8` arms ascending.
//! - `declare_descriptors!` (`descriptors/any.rs`), `declare_extension_bodies!`
//!   (`extension/mod.rs`), `declare_tables!` (`tables/any.rs`, by range start) and
//!   `declare_payloads!` (`dvb-t2mi/src/payload/any.rs`): entries ascending.
//! - named `const` blocks: `mpeg-pes/src/stream_id.rs`, `transmux` stream_type /
//!   OTI / RTMP message-type / VVC NAL constants, `dvb-si` BIOP tags.
//!
//! **Intentionally NOT numeric** (spec-clause or wire-field order, grouped by
//! feature): the files in `ORDER_COMMENT_REQUIRED` must each carry a one-line
//! `// ORDER:` comment saying so; a missing comment fails the test, as does a
//! stale entry. Everything else outside this list is simply not covered.
//!
//! The test skips (passing, printing why) when the sibling crates are not on
//! disk, e.g. when run from a published `broadcast-common` package.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> Option<PathBuf> {
    let r = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .to_path_buf();
    r.join("dvb-si/src/tables/pmt.rs").exists().then_some(r)
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn lit(s: &str) -> u64 {
    let s = s.replace('_', "");
    if let Some(h) = s.strip_prefix("0x") {
        u64::from_str_radix(h, 16).unwrap()
    } else if let Some(b) = s.strip_prefix("0b") {
        u64::from_str_radix(b, 2).unwrap()
    } else {
        s.parse().unwrap()
    }
}

/// Leading numeric literal of `s` (after optional whitespace), if any.
fn leading_lit(s: &str) -> Option<(u64, &str)> {
    let t = s.trim_start();
    let end = t
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(t.len());
    let tok = &t[..end];
    if tok.starts_with(|c: char| c.is_ascii_digit()) {
        Some((lit(tok), &t[end..]))
    } else {
        None
    }
}

/// Text between the `{` following `marker` and its matching `}`.
fn braced<'a>(src: &'a str, marker: &str) -> &'a str {
    let at = src
        .find(marker)
        .unwrap_or_else(|| panic!("anchor not found: {marker}"));
    let open = at + src[at..].find('{').expect("no {");
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open + 1..open + i];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces after {marker}");
}

fn strictly_ascending(what: &str, vals: &[(usize, u64)]) {
    assert!(
        vals.len() >= 2,
        "{what}: found only {} entries (anchor drift?)",
        vals.len()
    );
    for w in vals.windows(2) {
        assert!(
            w[1].1 > w[0].1,
            "{what}: entry {:#x} (line {}) does not follow {:#x} (line {}) in ascending order",
            w[1].1,
            w[1].0,
            w[0].1,
            w[0].0
        );
    }
}

/// `Ident = <lit>` discriminants of `pub enum <name>`.
fn enum_discriminants(root: &Path, rel: &str, name: &str) {
    let src = read(root, rel);
    let body = braced(&src, &format!("pub enum {name}"));
    let vals: Vec<_> = body
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let t = l.trim();
            let (id, rest) = t.split_once('=')?;
            if !id
                .trim()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                || id.trim().is_empty()
            {
                return None;
            }
            leading_lit(rest).map(|(v, _)| (i + 1, v))
        })
        .collect();
    strictly_ascending(&format!("{rel} enum {name} discriminants"), &vals);
}

/// Arms of `fn <func>` inside the `impl_marker` block: `0xNN[..=0xMM] =>`
/// (decode) or `Self::X => 0xNN` (encode), in source order. Catch-all arms
/// (`_`, bare identifiers) carry no literal and are skipped.
fn fn_arms(root: &Path, rel: &str, impl_marker: &str, func: &str) -> Vec<(usize, u64)> {
    let src = read(root, rel);
    let at = src
        .find(impl_marker)
        .unwrap_or_else(|| panic!("{rel}: {impl_marker}"));
    let imp = &src[at..];
    let body = braced(imp, &format!("fn {func}("));
    let mut out = Vec::new();
    for (i, l) in body.lines().enumerate() {
        let t = l.trim();
        if let Some((v, rest)) = leading_lit(t) {
            // decode arm: `0x00 =>`, `0x36..=0x7E =>`, or `0x01 | 0x02 =>`
            let rest = rest.trim_start();
            if rest.starts_with("=>") || rest.starts_with("..") || rest.starts_with('|') {
                out.push((i + 1, v));
            }
        } else if t.starts_with("Self::") {
            if let Some((_, rhs)) = t.split_once("=>") {
                if let Some((v, tail)) = leading_lit(rhs) {
                    if tail.trim_start().starts_with(',') {
                        out.push((i + 1, v));
                    }
                }
            }
        }
    }
    out
}

fn arms_ascending(root: &Path, rel: &str, impl_marker: &str, func: &str) {
    let v = fn_arms(root, rel, impl_marker, func);
    strictly_ascending(&format!("{rel} {impl_marker}::{func} arms"), &v);
}

/// Identifiers `Self::X` starting arms inside `fn <func>`, in order.
fn arm_idents(src: &str, impl_marker: &str, func: &str) -> Vec<String> {
    let at = src.find(impl_marker).expect(impl_marker);
    let body = braced(&src[at..], &format!("fn {func}("));
    body.lines()
        .filter_map(|l| {
            let t = l.trim();
            let rest = t.strip_prefix("Self::")?;
            if !t.contains("=>") {
                return None;
            }
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            Some(id)
        })
        .collect()
}

fn enum_variant_idents(src: &str, name: &str) -> Vec<String> {
    let body = braced(src, &format!("pub enum {name}"));
    let mut depth = 0i32;
    let mut out = Vec::new();
    for l in body.lines() {
        let t = l.trim();
        if depth == 0 && t.starts_with(|c: char| c.is_ascii_uppercase()) {
            let id: String = t
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            out.push(id);
        }
        depth += l.matches(['(', '{']).count() as i32 - l.matches([')', '}']).count() as i32;
    }
    out
}

/// `const NAME: uN = <lit>;` between the first line containing `start` and the
/// last line containing `end`.
fn consts_ascending(root: &Path, rel: &str, start: &str, end: &str) {
    let src = read(root, rel);
    let lines: Vec<&str> = src.lines().collect();
    let s = lines
        .iter()
        .position(|l| l.contains(start))
        .unwrap_or_else(|| panic!("{rel}: start anchor {start}"));
    let e = lines
        .iter()
        .rposition(|l| l.contains(end))
        .unwrap_or_else(|| panic!("{rel}: end anchor {end}"));
    assert!(e > s, "{rel}: end anchor before start");
    let vals: Vec<_> = lines[s..=e]
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let t = l.trim();
            let t = t
                .strip_prefix("pub(crate) ")
                .or(t.strip_prefix("pub "))
                .unwrap_or(t);
            let t = t.strip_prefix("const ")?;
            let (_, rhs) = t.split_once('=')?;
            let (v, tail) = leading_lit(rhs)?;
            tail.trim_start().starts_with(';').then_some((s + i + 1, v))
        })
        .collect();
    strictly_ascending(&format!("{rel} consts {start}..{end}"), &vals);
}

/// `Name = <lit> =>` entries (or `Name = [lo..=hi] =>` by `lo`) after `marker`.
fn declare_list(root: &Path, rel: &str, marker: &str) {
    let src = read(root, rel);
    let at = src
        .find(marker)
        .unwrap_or_else(|| panic!("{rel}: {marker}"));
    let body = braced(&src[at..], marker.trim_end_matches('{').trim_end());
    let vals: Vec<_> = body
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let t = l.trim();
            let (id, rest) = t.split_once('=')?;
            if id.trim().is_empty()
                || !id
                    .trim()
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return None;
            }
            let rest = rest.trim_start().trim_start_matches('[');
            let (v, tail) = leading_lit(rest)?;
            let tail = tail.trim_start();
            (tail.starts_with("=>") || tail.starts_with("..")).then_some((i + 1, v))
        })
        .collect();
    strictly_ascending(&format!("{rel} {marker} list"), &vals);
}

/// `/// 0xNN` doc rows of an enum, ascending (ranges use their start).
fn enum_doc_rows(root: &Path, rel: &str, name: &str) {
    let src = read(root, rel);
    let body = braced(&src, &format!("pub enum {name}"));
    let vals: Vec<_> = body
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let t = l.trim().strip_prefix("///")?.trim_start();
            let t = t.trim_start_matches('`');
            t.starts_with("0x")
                .then(|| leading_lit(t).map(|(v, _)| (i + 1, v)))?
        })
        .collect();
    strictly_ascending(&format!("{rel} enum {name} doc rows"), &vals);
}

#[test]
fn discriminant_enums_ascending() {
    let Some(r) = root() else {
        eprintln!("ordering_guard: sibling crates not present; skipped");
        return;
    };
    enum_discriminants(&r, "dvb-si/src/descriptor_tag.rs", "DescriptorTag");
    enum_discriminants(&r, "dvb-si/src/table_id.rs", "TableId");
    enum_discriminants(
        &r,
        "dvb-si/src/descriptors/extension/mod.rs",
        "ExtensionTag",
    );
}

#[test]
fn stream_type_tables_ascending_and_mirrored() {
    let Some(r) = root() else {
        eprintln!("ordering_guard: sibling crates not present; skipped");
        return;
    };
    let f = "dvb-si/src/tables/pmt.rs";
    arms_ascending(&r, f, "impl StreamType", "from_u8");
    arms_ascending(&r, f, "impl StreamType", "to_u8");
    enum_doc_rows(&r, f, "StreamType");
    // The enum, `to_u8` and `name()` list the named variants in the same order.
    let src = read(&r, f);
    let variants = enum_variant_idents(&src, "StreamType");
    for func in ["to_u8", "name"] {
        let arms = arm_idents(&src, "impl StreamType", func);
        let mut idx = 0usize;
        for a in &arms {
            let pos = variants[idx..]
                .iter()
                .position(|v| v == a)
                .unwrap_or_else(|| panic!("{f}: `{func}` arm Self::{a} out of enum order"));
            idx += pos + 1;
        }
    }
}

#[test]
fn registry_enums_arms_ascending() {
    let Some(r) = root() else {
        eprintln!("ordering_guard: sibling crates not present; skipped");
        return;
    };
    for func in ["from_u8", "to_u8"] {
        arms_ascending(
            &r,
            "dvb-si/src/descriptors/service.rs",
            "impl ServiceType",
            func,
        );
        arms_ascending(
            &r,
            "dvb-si/src/descriptors/linkage.rs",
            "impl LinkageType",
            func,
        );
    }
}

#[test]
fn declare_lists_ascending() {
    let Some(r) = root() else {
        eprintln!("ordering_guard: sibling crates not present; skipped");
        return;
    };
    declare_list(
        &r,
        "dvb-si/src/descriptors/any.rs",
        "declare_descriptors! {",
    );
    declare_list(
        &r,
        "dvb-si/src/descriptors/extension/mod.rs",
        "declare_extension_bodies! {",
    );
    declare_list(&r, "dvb-si/src/tables/any.rs", "declare_tables! {");
    declare_list(&r, "dvb-t2mi/src/payload/any.rs", "declare_payloads! {");
}

#[test]
fn named_const_blocks_ascending() {
    let Some(r) = root() else {
        eprintln!("ordering_guard: sibling crates not present; skipped");
        return;
    };
    consts_ascending(
        &r,
        "mpeg-pes/src/stream_id.rs",
        "const PROGRAM_STREAM_MAP",
        "const PROGRAM_STREAM_DIRECTORY",
    );
    consts_ascending(
        &r,
        "transmux/src/ts_demux.rs",
        "const STREAM_TYPE_MPEG2_VIDEO",
        "const STREAM_TYPE_METADATA_PES",
    );
    consts_ascending(
        &r,
        "transmux/src/ts_demux.rs",
        "const STREAM_TYPE_PRIVATE_SECTIONS",
        "const STREAM_TYPE_SCTE35",
    );
    consts_ascending(
        &r,
        "transmux/src/ts_demux.rs",
        "const OTI_MPEG4_AUDIO",
        "const OTI_MPEG2_AUDIO",
    );
    consts_ascending(
        &r,
        "transmux/src/ts_mux.rs",
        "const STREAM_TYPE_AVC",
        "const STREAM_TYPE_MPEGH",
    );
    consts_ascending(
        &r,
        "transmux/src/rtmp.rs",
        "pub const SET_CHUNK_SIZE",
        "pub const COMMAND_AMF0",
    );
    consts_ascending(
        &r,
        "transmux/src/au.rs",
        "const VVC_VCL_MAX",
        "const VVC_FD",
    );
    consts_ascending(
        &r,
        "dvb-si/src/carousel/biop/mod.rs",
        "pub const TAG_BIOP",
        "pub const TAG_SERVICE_LOCATION",
    );
}

/// Files whose tables are deliberately ordered by spec clause / wire field
/// rather than numerically; each must say so in a `// ORDER:` comment.
const ORDER_COMMENT_REQUIRED: &[(&str, &str)] = &[
    ("transmux/src/webm_demux.rs", "EBML element-hierarchy order"),
    (
        "st377-1/src/identification.rs",
        "MXF Annex A.3 property-table order",
    ),
    (
        "st377-1/src/preface.rs",
        "MXF Annex A.2 property-table order",
    ),
    ("st377-1/src/track.rs", "MXF Annex B property-table order"),
    (
        "dvb-t2mi/src/payload/l1/post.rs",
        "L1-post wire field order (bit widths)",
    ),
    (
        "mpeg-pes/src/stream_id.rs",
        "stream_id_extension is a separate namespace",
    ),
];

#[test]
fn intentionally_unsorted_tables_are_documented() {
    let Some(r) = root() else {
        eprintln!("ordering_guard: sibling crates not present; skipped");
        return;
    };
    for (rel, why) in ORDER_COMMENT_REQUIRED {
        assert!(
            read(&r, rel)
                .lines()
                .any(|l| l.trim_start().starts_with("// ORDER:")),
            "{rel} ({why}) must carry a one-line `// ORDER:` comment explaining its non-numeric order"
        );
    }
}
