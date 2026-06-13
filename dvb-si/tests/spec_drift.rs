//! Drift tests: assert that `spec_tables/*.toml` and the Rust enum are in sync.
//!
//! For each enum a byte-sweep produces a "from code" set and the TOML parser
//! produces a "from spec" set.  The test fails (symmetrically) if either set
//! has something the other lacks, so it catches additions, removals, and
//! renumbers in either direction.

use dvb_si::{descriptor_tag::DescriptorTag, table_id::TableId, tables::pmt::StreamType};
use std::collections::{BTreeMap, BTreeSet};

// ── tiny TOML parser ─────────────────────────────────────────────────────────

/// Parse the spec_tables TOML format, returning `(value, variant, spec)`.
///
/// Only handles the fixed schema used in this project:
/// - blank lines and `#`-comment lines are skipped
/// - `[[entry]]` starts a new record
/// - `key = 0xNN` / `key = "..."` lines populate the record
fn parse_entries(toml: &str) -> Vec<(u8, String, String)> {
    let mut results = Vec::new();
    let mut cur_value: Option<u8> = None;
    let mut cur_variant: Option<String> = None;
    let mut cur_spec: Option<String> = None;

    let flush = |v: &mut Option<u8>,
                 var: &mut Option<String>,
                 sp: &mut Option<String>,
                 out: &mut Vec<(u8, String, String)>| {
        if let (Some(value), Some(variant), Some(spec)) = (v.take(), var.take(), sp.take()) {
            out.push((value, variant, spec));
        }
    };

    for raw in toml.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[entry]]" {
            flush(
                &mut cur_value,
                &mut cur_variant,
                &mut cur_spec,
                &mut results,
            );
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim();
            let val = line[eq + 1..].trim();
            match key {
                "value" => {
                    let hex = val.trim_start_matches("0x").trim_start_matches("0X");
                    cur_value = Some(
                        u8::from_str_radix(hex, 16)
                            .unwrap_or_else(|_| panic!("bad hex in TOML: {val:?}")),
                    );
                }
                "variant" => {
                    cur_variant = Some(val.trim_matches('"').replace("\\\"", "\"").to_string());
                }
                "spec" => {
                    cur_spec = Some(val.trim_matches('"').replace("\\\"", "\"").to_string());
                }
                _ => {} // ignore unknown keys
            }
        }
    }
    // flush the last record
    flush(
        &mut cur_value,
        &mut cur_variant,
        &mut cur_spec,
        &mut results,
    );
    results
}

// ── test: TableId ─────────────────────────────────────────────────────────────

#[test]
fn table_id_toml_matches_enum() {
    let toml = include_str!("../spec_tables/table_id.toml");
    let entries = parse_entries(toml);

    // build set from TOML
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // build set from enum: byte sweep 0..=255
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        if let Ok(id) = TableId::try_from(b) {
            code_set.insert((b, format!("{id:?}")));
        }
    }

    // tripwire
    assert_eq!(
        code_set.len(),
        29,
        "TableId byte sweep produced {} variants, expected 29",
        code_set.len()
    );

    // symmetric diff
    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "TableId drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: DescriptorTag ───────────────────────────────────────────────────────

#[test]
fn descriptor_tag_toml_matches_enum() {
    let toml = include_str!("../spec_tables/descriptor_tag.toml");
    let entries = parse_entries(toml);

    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // build set from enum: byte sweep 0..=255 via TryFrom<u8>
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        if let Ok(tag) = DescriptorTag::try_from(b) {
            code_set.insert((b, format!("{tag:?}")));
        }
    }

    // tripwire: 17 MPEG + 64 DVB = 81
    assert_eq!(
        code_set.len(),
        81,
        "DescriptorTag list has {} entries, expected 81",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "DescriptorTag drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: StreamType ─────────────────────────────────────────────────────────

#[test]
fn stream_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/stream_type.toml");
    let entries = parse_entries(toml);

    // build TOML set and the value→spec map for the name check
    let mut toml_set: BTreeSet<(u8, String)> = BTreeSet::new();
    let mut toml_spec: BTreeMap<u8, String> = BTreeMap::new();
    for (v, var, spec) in &entries {
        toml_set.insert((*v, var.clone()));
        toml_spec.insert(*v, spec.clone());
    }

    // byte sweep: skip catch-alls
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let st = StreamType::from_u8(b);
        let is_catch_all = matches!(
            st,
            StreamType::ReservedRange(_) | StreamType::UserPrivate(_)
        );
        if is_catch_all {
            continue;
        }
        let debug_name = format!("{st:?}");
        // for named variants also assert TOML spec == .name()
        if let Some(toml_sp) = toml_spec.get(&b) {
            assert_eq!(
                toml_sp,
                st.name(),
                "StreamType 0x{b:02X} ({debug_name}): TOML spec {toml_sp:?} != .name() {:?}",
                st.name()
            );
        }
        code_set.insert((st.to_u8(), debug_name));
    }

    // tripwire: 54 named standard values (0x00..=0x35) + 0x7F + 0x81 + 0x86 + 0x87 = 58
    assert_eq!(
        code_set.len(),
        58,
        "StreamType sweep produced {} named variants, expected 58",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "StreamType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}
