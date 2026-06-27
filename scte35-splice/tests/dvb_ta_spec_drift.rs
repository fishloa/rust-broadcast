//! Drift tests for the DVB-TA coded enums: `EquivalentSegmentationType` (Table 2)
//! and `TimelineType` (Table 4).
//!
//! Each test byte-sweeps the enum's domain, builds the "from code" set (skipping
//! the Reserved(_) catch-all), then compares against the "from spec" set loaded
//! from the TOML mirror in `docs/enums/dvb_ta/`. Fails symmetrically if either
//! set has something the other lacks.

use scte35_splice::dvb_ta::{EquivalentSegmentationType, TimelineType};
use std::collections::BTreeSet;

// ── tiny TOML parser (mirrors spec_drift.rs) ─────────────────────────────────

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

// ── test: EquivalentSegmentationType ─────────────────────────────────────────

#[test]
fn equivalent_segmentation_type_toml_matches_enum() {
    let toml = include_str!("../docs/enums/dvb_ta/equivalent_segmentation_type.toml");
    let entries = parse_entries(toml);

    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // byte-sweep the 4-bit domain (0x0..=0xF); skip the Reserved(_) catch-all
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=0x0F {
        let t = EquivalentSegmentationType::from_bits(b);
        if matches!(t, EquivalentSegmentationType::Reserved(_)) {
            continue;
        }
        code_set.insert((t.bits(), format!("{t:?}")));
    }

    // tripwire: 5 named variants (0x0–0x4, Table 2)
    assert_eq!(
        code_set.len(),
        5,
        "EquivalentSegmentationType sweep produced {} named variants, expected 5",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "EquivalentSegmentationType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: TimelineType ────────────────────────────────────────────────────────

#[test]
fn timeline_type_toml_matches_enum() {
    let toml = include_str!("../docs/enums/dvb_ta/timeline_type.toml");
    let entries = parse_entries(toml);

    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // byte-sweep the 4-bit domain (0x0..=0xF); skip the Reserved(_) catch-all
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=0x0F {
        let t = TimelineType::from_bits(b);
        if matches!(t, TimelineType::Reserved(_)) {
            continue;
        }
        code_set.insert((t.bits(), format!("{t:?}")));
    }

    // tripwire: 3 named variants (0x0–0x2, Table 4)
    assert_eq!(
        code_set.len(),
        3,
        "TimelineType sweep produced {} named variants, expected 3",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "TimelineType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}
