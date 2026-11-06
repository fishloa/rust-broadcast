//! Drift tests for the DVB-TA coded enums: `EquivalentSegmentationType` (Table 2)
//! and `TimelineType` (Table 4).
//!
//! Each test byte-sweeps the enum's domain, builds the "from code" set (skipping
//! the Reserved(_) catch-all), then compares against the "from spec" set loaded
//! from the TOML mirror in `private/specs-verified/scte/scte35-splice/enums/dvb_ta/`. Fails symmetrically if either
//! set has something the other lacks.

use scte35_splice::dvb_ta::{EquivalentSegmentationType, TimelineType};
use std::collections::BTreeSet;

// ── spec mirror loader ───────────────────────────────────────────────────────

fn spec_toml(rel: &str) -> Option<String> {
    test_fixtures::spec_string(
        &format!("scte/scte35-splice/enums/{rel}"),
        "scte35-splice dvb_ta_spec_drift",
    )
}

// ── test: EquivalentSegmentationType ─────────────────────────────────────────

#[test]
fn equivalent_segmentation_type_toml_matches_enum() {
    let Some(toml) = spec_toml("dvb_ta/equivalent_segmentation_type.toml") else {
        return;
    };
    let toml = toml.as_str();
    let entries = test_fixtures::parse_spec_entries::<u8>(toml);

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
    let Some(toml) = spec_toml("dvb_ta/timeline_type.toml") else {
        return;
    };
    let toml = toml.as_str();
    let entries = test_fixtures::parse_spec_entries::<u8>(toml);

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
