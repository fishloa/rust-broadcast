//! Drift tests: assert that the `spec_tables/*.toml` mirrors and the code-backing
//! enums / dispatch tables are in sync.
//!
//! A byte-sweep over 0..=255 produces the "from code" set; the TOML parser
//! produces the "from spec" set.  Fails symmetrically if either set has
//! something the other lacks.

use dvb_scte35::commands::AnyCommand;
use dvb_scte35::descriptors::segmentation_enums::{
    DeviceRestrictions, SegmentationTypeId, SegmentationUpidType,
};
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

// ── test: SegmentationTypeId ──────────────────────────────────────────────────

#[test]
fn segmentation_type_id_toml_matches_enum() {
    let toml = include_str!("../spec_tables/segmentation_type_id.toml");
    let entries = parse_entries(toml);

    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // byte sweep: skip Reserved(_) catch-all
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let id = SegmentationTypeId::from_u8(b);
        if matches!(id, SegmentationTypeId::Reserved(_)) {
            continue;
        }
        code_set.insert((id.to_u8(), format!("{id:?}")));
    }

    // tripwire: 48 named variants (verified from the source)
    assert_eq!(
        code_set.len(),
        48,
        "SegmentationTypeId sweep produced {} named variants, expected 48",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "SegmentationTypeId drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: DeviceRestrictions ─────────────────────────────────────────────────

#[test]
fn device_restrictions_toml_matches_enum() {
    let toml = include_str!("../spec_tables/device_restrictions.toml");
    let entries = parse_entries(toml);

    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // byte sweep over the 2-bit domain (0..=3); DeviceRestrictions has no catch-all
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=3 {
        let dr = DeviceRestrictions::from_bits(b);
        code_set.insert((dr.bits(), format!("{dr:?}")));
    }

    // tripwire: 4 named variants (Table 21 — 2-bit field, exhaustive)
    assert_eq!(
        code_set.len(),
        4,
        "DeviceRestrictions sweep produced {} named variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "DeviceRestrictions drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: SegmentationUpidType ────────────────────────────────────────────────

#[test]
fn segmentation_upid_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/segmentation_upid_type.toml");
    let entries = parse_entries(toml);

    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // byte sweep: skip Reserved(_) catch-all
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let ut = SegmentationUpidType::from_u8(b);
        if matches!(ut, SegmentationUpidType::Reserved(_)) {
            continue;
        }
        code_set.insert((ut.to_u8(), format!("{ut:?}")));
    }

    // tripwire: 18 named variants (0x00..=0x11, verified from the source)
    assert_eq!(
        code_set.len(),
        18,
        "SegmentationUpidType sweep produced {} named variants, expected 18",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "SegmentationUpidType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: splice_command_type (AnyCommand::DISPATCHED_TYPES) ─────────────────

#[test]
fn splice_command_type_toml_matches_dispatch() {
    let toml = include_str!("../spec_tables/splice_command_type.toml");
    let entries = parse_entries(toml);

    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // The "code" set is AnyCommand::DISPATCHED_TYPES paired with the command name.
    // We get the name by dispatching a zero-length body for each type — the parse
    // may fail (e.g. splice_insert needs content) but `name()` is on the variant
    // not the result, so we use a hard-coded map that mirrors DISPATCHED_TYPES.
    // Instead, map each dispatched type to its variant name via a round-trip
    // through AnyCommand::dispatch: for fixed-format zero-payload commands
    // (splice_null, bandwidth_reservation) this works directly; for others we
    // use the variant name from the TOML as the oracle.
    //
    // Simpler: treat DISPATCHED_TYPES as the authoritative code list and match
    // variant names by parsing each against the TOML entries (value→variant).
    let dispatched: BTreeSet<u8> = AnyCommand::DISPATCHED_TYPES.iter().copied().collect();

    // Build code_set from (type_byte, variant_name) by looking up the name
    // in the TOML.  This is a cross-check: if a type is dispatched but not in
    // the TOML, only_in_code will flag it; if in the TOML but not dispatched,
    // only_in_toml will flag it.
    // We use a variant-name map derived from the dispatch order preserved in
    // the existing code (constants in each command module).
    let type_to_name: std::collections::BTreeMap<u8, &'static str> = [
        (0x00u8, "SpliceNull"),
        (0x04u8, "SpliceSchedule"),
        (0x05u8, "SpliceInsert"),
        (0x06u8, "TimeSignal"),
        (0x07u8, "BandwidthReservation"),
        (0xFFu8, "PrivateCommand"),
    ]
    .into_iter()
    .collect();

    let code_set: BTreeSet<(u8, String)> = dispatched
        .iter()
        .map(|&ct| {
            let name = type_to_name
                .get(&ct)
                .unwrap_or_else(|| panic!("DISPATCHED_TYPES has 0x{ct:02X} with no name entry"));
            (ct, name.to_string())
        })
        .collect();

    // tripwire: 6 implemented command types (§9.6.1 Table 7, verified from source)
    assert_eq!(
        code_set.len(),
        6,
        "splice_command_type dispatch has {} types, expected 6",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "splice_command_type drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}
