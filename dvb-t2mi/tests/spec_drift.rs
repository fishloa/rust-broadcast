//! Drift tests: assert that each `spec_tables/*.toml` mirror and its
//! code-backing enum/dispatch table are in sync.
//!
//! A byte-sweep over the relevant domain produces the "from code" set; the
//! TOML parser produces the "from spec" set.  Fails symmetrically if either
//! set has something the other lacks.

use dvb_t2mi::packet::PacketType;
use dvb_t2mi::payload::fef_null::S1Field;
use dvb_t2mi::payload::fef_subpart::SubpartVariety;
use dvb_t2mi::payload::individual_addressing::AddressingFunctionTag;
use dvb_t2mi::payload::l1_current::FrequencySource;
use dvb_t2mi::payload::timestamp::Bandwidth;
use dvb_t2mi::payload::{
    AuxStreamType, GuardInterval, L1CodeRate, L1FecType, L1Modulation, PilotPattern, PlpFecType,
    PlpMode, PlpModulation, PlpPayloadType, PlpType, T2Version, TxInputStreamType,
};
use std::collections::BTreeSet;

// ── tiny TOML parser ─────────────────────────────────────────────────────────

/// Parse the spec_tables TOML format, returning `(value_u16, variant, spec)`.
fn parse_entries(toml: &str) -> Vec<(u16, String, String)> {
    let mut results = Vec::new();
    let mut cur_value: Option<u16> = None;
    let mut cur_variant: Option<String> = None;
    let mut cur_spec: Option<String> = None;

    let flush = |v: &mut Option<u16>,
                 var: &mut Option<String>,
                 sp: &mut Option<String>,
                 out: &mut Vec<(u16, String, String)>| {
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
                        u16::from_str_radix(hex, 16)
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

// ── helpers ───────────────────────────────────────────────────────────────────

fn toml_set_u8(toml: &str) -> BTreeSet<(u8, String)> {
    parse_entries(toml)
        .iter()
        .map(|(v, var, _)| (*v as u8, var.clone()))
        .collect()
}

fn toml_set_u16(toml: &str) -> BTreeSet<(u16, String)> {
    parse_entries(toml)
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect()
}

// ── test: PacketType ─────────────────────────────────────────────────────────

#[test]
fn packet_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/packet_type.toml");
    let toml_set = toml_set_u8(toml);

    // byte sweep: TryFromPrimitive — skip bytes that don't map to a variant
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        if let Ok(pt) = PacketType::try_from(b) {
            code_set.insert((pt as u8, format!("{pt:?}")));
        }
    }

    // tripwire: 12 named variants (Table 1, verified from source)
    assert_eq!(
        code_set.len(),
        12,
        "PacketType sweep produced {} named variants, expected 12",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "PacketType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: AddressingFunctionTag ───────────────────────────────────────────────

#[test]
fn addressing_function_tag_toml_matches_enum() {
    let toml = include_str!("../spec_tables/addressing_function_tag.toml");
    let toml_set = toml_set_u8(toml);

    // byte sweep: TryFromPrimitive — skip bytes that don't map to a variant
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        if let Ok(tag) = AddressingFunctionTag::try_from(b) {
            code_set.insert((tag as u8, format!("{tag:?}")));
        }
    }

    // tripwire: 14 named variants (Tables 5 & 6, verified from source)
    assert_eq!(
        code_set.len(),
        14,
        "AddressingFunctionTag sweep produced {} named variants, expected 14",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "AddressingFunctionTag drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: Bandwidth ───────────────────────────────────────────────────────────

#[test]
fn bandwidth_toml_matches_enum() {
    let toml = include_str!("../spec_tables/bandwidth.toml");
    let toml_set = toml_set_u8(toml);

    // byte sweep: TryFromPrimitive — skip bytes that don't map to a variant
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        if let Ok(bw) = Bandwidth::try_from(b) {
            code_set.insert((bw as u8, format!("{bw:?}")));
        }
    }

    // tripwire: 6 named variants (Table 3, verified from source)
    assert_eq!(
        code_set.len(),
        6,
        "Bandwidth sweep produced {} named variants, expected 6",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "Bandwidth drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: FrequencySource ─────────────────────────────────────────────────────

#[test]
fn frequency_source_toml_matches_enum() {
    let toml = include_str!("../spec_tables/frequency_source.toml");
    let toml_set = toml_set_u8(toml);

    // byte sweep: TryFromPrimitive — skip bytes that don't map to a variant
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        if let Ok(fs) = FrequencySource::try_from(b) {
            code_set.insert((fs as u8, format!("{fs:?}")));
        }
    }

    // tripwire: 3 named variants (Table 2, verified from source)
    assert_eq!(
        code_set.len(),
        3,
        "FrequencySource sweep produced {} named variants, expected 3",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "FrequencySource drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: S1Field ─────────────────────────────────────────────────────────────

#[test]
fn s1_field_toml_matches_enum() {
    let toml = include_str!("../spec_tables/s1_field.toml");
    let toml_set = toml_set_u8(toml);

    // byte sweep: S1Field is exhaustive over 3-bit domain; TryFromPrimitive
    // accepts 0..=7, rejects 8..=255
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        if let Ok(s1) = S1Field::try_from(b) {
            code_set.insert((s1 as u8, format!("{s1:?}")));
        }
    }

    // tripwire: 8 named variants (Table 18, 3-bit field exhaustive)
    assert_eq!(
        code_set.len(),
        8,
        "S1Field sweep produced {} named variants, expected 8",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "S1Field drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: SubpartVariety ──────────────────────────────────────────────────────

#[test]
fn subpart_variety_toml_matches_enum() {
    let toml = include_str!("../spec_tables/subpart_variety.toml");
    let toml_set = toml_set_u16(toml);

    // u16 sweep over 0..=255 (all defined values fit in this range);
    // SubpartVariety is TryFromPrimitive on u16; values 4..=0xFFFF return Err.
    let mut code_set: BTreeSet<(u16, String)> = BTreeSet::new();
    for b in 0u16..=255 {
        if let Ok(sv) = SubpartVariety::try_from(b) {
            code_set.insert((sv as u16, format!("{sv:?}")));
        }
    }

    // tripwire: 4 named variants (Table 13, verified from source)
    assert_eq!(
        code_set.len(),
        4,
        "SubpartVariety sweep produced {} named variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "SubpartVariety drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── L1 signalling enum drift tests ───────────────────────────────────────────

macro_rules! l1_drift_test {
    ($test_name:ident, $toml_file:literal, $enum_ty:ident, $domain:expr, $expected:expr, $reserved_pat:pat) => {
        #[test]
        fn $test_name() {
            let toml = include_str!(concat!("../spec_tables/", $toml_file));
            let toml_set: BTreeSet<(u8, String)> = parse_entries(toml)
                .iter()
                .map(|(v, var, _)| (*v as u8, var.clone()))
                .collect();

            let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
            for b in $domain {
                let e = $enum_ty::from_u8(b);
                if !matches!(e, $reserved_pat) {
                    code_set.insert((e.to_u8(), format!("{e:?}")));
                }
            }

            assert_eq!(
                code_set.len(),
                $expected,
                "{}: expected {} named variants, got {}",
                stringify!($enum_ty),
                $expected,
                code_set.len()
            );
            let only_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
            let only_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
            assert!(
                only_toml.is_empty() && only_code.is_empty(),
                "{} drift!\n  only in TOML: {only_toml:?}\n  only in code: {only_code:?}",
                stringify!($enum_ty)
            );
        }
    };
}

l1_drift_test!(
    tx_input_stream_type_drift,
    "tx_input_stream_type.toml",
    TxInputStreamType,
    0u8..=0xFF,
    3,
    TxInputStreamType::Reserved(_)
);

l1_drift_test!(
    guard_interval_drift,
    "guard_interval.toml",
    GuardInterval,
    0u8..=7,
    7,
    GuardInterval::Reserved(_)
);

l1_drift_test!(
    l1_modulation_drift,
    "l1_modulation.toml",
    L1Modulation,
    0u8..=0x0F,
    4,
    L1Modulation::Reserved(_)
);

l1_drift_test!(
    l1_code_rate_drift,
    "l1_code_rate.toml",
    L1CodeRate,
    0u8..=3,
    1,
    L1CodeRate::Reserved(_)
);

l1_drift_test!(
    l1_fec_type_drift,
    "l1_fec_type.toml",
    L1FecType,
    0u8..=3,
    1,
    L1FecType::Reserved(_)
);

l1_drift_test!(
    pilot_pattern_drift,
    "pilot_pattern.toml",
    PilotPattern,
    0u8..=0x0F,
    8,
    PilotPattern::Reserved(_)
);

l1_drift_test!(
    t2_version_drift,
    "t2_version.toml",
    T2Version,
    0u8..=0x0F,
    3,
    T2Version::Reserved(_)
);

l1_drift_test!(
    plp_type_drift,
    "plp_type.toml",
    PlpType,
    0u8..=7,
    3,
    PlpType::Reserved(_)
);

l1_drift_test!(
    plp_payload_type_drift,
    "plp_payload_type.toml",
    PlpPayloadType,
    0u8..=0x1F,
    4,
    PlpPayloadType::Reserved(_)
);

l1_drift_test!(
    plp_modulation_drift,
    "plp_modulation.toml",
    PlpModulation,
    0u8..=7,
    4,
    PlpModulation::Reserved(_)
);

l1_drift_test!(
    plp_fec_type_drift,
    "plp_fec_type.toml",
    PlpFecType,
    0u8..=3,
    2,
    PlpFecType::Reserved(_)
);

l1_drift_test!(
    plp_mode_drift,
    "plp_mode.toml",
    PlpMode,
    0u8..=3,
    3,
    PlpMode::Reserved(_)
);

l1_drift_test!(
    aux_stream_type_drift,
    "aux_stream_type.toml",
    AuxStreamType,
    0u8..=0x0F,
    1,
    AuxStreamType::Reserved(_)
);
