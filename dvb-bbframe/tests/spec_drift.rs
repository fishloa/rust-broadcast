//! Drift tests: assert that each `spec_tables/*.toml` mirror and its
//! code-backing enum are in sync.
//!
//! A byte-sweep over the relevant domain produces the "from code" set; the
//! TOML parser produces the "from spec" set.  Fails symmetrically if either
//! set has something the other lacks.

use dvb_bbframe::header::TsGs;
use dvb_bbframe::issy::BufsUnit;
use std::collections::BTreeSet;

// ── tiny TOML parser ─────────────────────────────────────────────────────────

/// Parse the spec_tables TOML format, returning `(value, variant, spec)`.
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
                _ => {}
            }
        }
    }
    flush(
        &mut cur_value,
        &mut cur_variant,
        &mut cur_spec,
        &mut results,
    );
    results
}

// ── test: TsGs ────────────────────────────────────────────────────────────────

#[test]
fn ts_gs_toml_matches_enum() {
    let toml = include_str!("../docs/enums/en_302_755/ts_gs.toml");
    let entries = parse_entries(toml);

    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // byte sweep over the 2-bit domain (0..=3); TsGs is exhaustive (no catch-all)
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=3 {
        if let Ok(ts_gs) = TsGs::try_from(b) {
            code_set.insert((ts_gs as u8, format!("{ts_gs:?}")));
        }
    }

    // tripwire: 4 named variants (Table 1, 2-bit field, exhaustive)
    assert_eq!(
        code_set.len(),
        4,
        "TsGs sweep produced {} named variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "TsGs drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: BufsUnit ────────────────────────────────────────────────────────────

#[test]
fn bufs_unit_toml_matches_enum() {
    let toml = include_str!("../docs/enums/en_302_755/bufs_unit.toml");
    let entries = parse_entries(toml);

    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // byte sweep over the 2-bit domain (0..=3); BufsUnit uses from_u8/to_u8
    // with masking — all 4 values are named (no catch-all)
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=3 {
        let bu = BufsUnit::from_u8(b);
        code_set.insert((bu.to_u8(), format!("{bu:?}")));
    }

    // tripwire: 4 named variants (Annex C Table C.1, 2-bit field, exhaustive)
    assert_eq!(
        code_set.len(),
        4,
        "BufsUnit sweep produced {} named variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "BufsUnit drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}
