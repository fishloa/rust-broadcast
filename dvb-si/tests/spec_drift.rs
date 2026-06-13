//! Drift tests: assert that `spec_tables/*.toml` and the Rust enum are in sync.
//!
//! For each enum a byte-sweep produces a "from code" set and the TOML parser
//! produces a "from spec" set.  The test fails (symmetrically) if either set
//! has something the other lacks, so it catches additions, removals, and
//! renumbers in either direction.

use dvb_si::{
    compatibility::DescriptorType,
    descriptor_tag::DescriptorTag,
    descriptors::{
        ac3::Ac3ServiceType,
        announcement_support::AnnouncementType,
        cable_delivery_system::FecOuter,
        content_identifier::CridType,
        data_stream_alignment::AlignmentType,
        extension::{
            Ac4ChannelMode, C2TuningFrequencyType, ExtensionTag, S2XMode, ShDiversityMode,
            T2SisoMiso, UriLinkageType,
        },
        fta_content_management::ControlRemoteAccess,
        iso_639_language::AudioType,
        linkage::LinkageType,
        s2_satellite_delivery_system::TsGsMode,
        satellite_delivery_system::Polarization,
        scrambling::ScramblingMode,
        service::ServiceType,
        subtitling::SubtitlingType,
        teletext::TeletextType,
        tva_id::TvaRunningStatus,
    },
    table_id::TableId,
    tables::{
        ait::ControlCode, int::IntActionType, pmt::StreamType, protection_message::ReferenceType,
        rct::LinkType, rnt::CridAuthorityPolicy, unt::UntActionType, RunningStatus,
    },
};
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

// ── test: ServiceType ─────────────────────────────────────────────────────────

#[test]
fn service_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/service_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = ServiceType::from_u8(b);
        if matches!(v, ServiceType::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        28,
        "ServiceType sweep produced {} variants, expected 28",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "ServiceType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: SubtitlingType ──────────────────────────────────────────────────────

#[test]
fn subtitling_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/subtitling_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = SubtitlingType::from_u8(b);
        if matches!(v, SubtitlingType::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        19,
        "SubtitlingType sweep produced {} variants, expected 19",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "SubtitlingType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: TeletextType ────────────────────────────────────────────────────────

#[test]
fn teletext_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/teletext_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = TeletextType::from_u8(b);
        if matches!(v, TeletextType::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        5,
        "TeletextType sweep produced {} variants, expected 5",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "TeletextType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: LinkageType ─────────────────────────────────────────────────────────

#[test]
fn linkage_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/linkage_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // Exclude catch-all variants: Reserved(u8) and ExtendedEventLinkage(u8) range.
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = LinkageType::from_u8(b);
        if matches!(
            v,
            LinkageType::Reserved(_) | LinkageType::ExtendedEventLinkage(_)
        ) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        15,
        "LinkageType sweep produced {} named variants, expected 15",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "LinkageType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: AudioType ───────────────────────────────────────────────────────────

#[test]
fn audio_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/audio_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // Exclude catch-all variants: UserPrivate(u8) and Reserved(u8).
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = AudioType::from_u8(b);
        if matches!(v, AudioType::UserPrivate(_) | AudioType::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        9,
        "AudioType sweep produced {} variants, expected 9",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "AudioType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: AnnouncementType ────────────────────────────────────────────────────

#[test]
fn announcement_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/announcement_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = AnnouncementType::from_u8(b);
        if matches!(v, AnnouncementType::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        8,
        "AnnouncementType sweep produced {} variants, expected 8",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "AnnouncementType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: AlignmentType ───────────────────────────────────────────────────────

#[test]
fn alignment_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/alignment_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = AlignmentType::from_u8(b);
        if matches!(v, AlignmentType::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        4,
        "AlignmentType sweep produced {} variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "AlignmentType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: Ac3ServiceType ──────────────────────────────────────────────────────

#[test]
fn ac3_service_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/ac3_service_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // 3-bit field: values 0-7 are all named; Unknown(u8) only fires for v>7 which
    // cannot occur from a wire 3-bit field but is excluded per catch-all rule.
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=7 {
        let v = Ac3ServiceType::from_u8(b);
        if matches!(v, Ac3ServiceType::Unknown(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        8,
        "Ac3ServiceType sweep produced {} variants, expected 8",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "Ac3ServiceType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: Ac4ChannelMode ──────────────────────────────────────────────────────

#[test]
fn ac4_channel_mode_toml_matches_enum() {
    let toml = include_str!("../spec_tables/ac4_channel_mode.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = Ac4ChannelMode::from_u8(b);
        if matches!(v, Ac4ChannelMode::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        3,
        "Ac4ChannelMode sweep produced {} variants, expected 3",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "Ac4ChannelMode drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: CridType ────────────────────────────────────────────────────────────

#[test]
fn crid_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/crid_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = CridType::from_u8(b);
        if matches!(v, CridType::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        4,
        "CridType sweep produced {} variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "CridType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: ControlRemoteAccess ─────────────────────────────────────────────────

#[test]
fn control_remote_access_toml_matches_enum() {
    let toml = include_str!("../spec_tables/control_remote_access.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // 2-bit field: all 4 values named; Reserved(u8) only for v>3.
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=3 {
        let v = ControlRemoteAccess::from_u8(b);
        if matches!(v, ControlRemoteAccess::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        4,
        "ControlRemoteAccess sweep produced {} variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "ControlRemoteAccess drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: ScramblingMode ──────────────────────────────────────────────────────

#[test]
fn scrambling_mode_toml_matches_enum() {
    let toml = include_str!("../spec_tables/scrambling_mode.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = ScramblingMode::from_u8(b);
        if matches!(v, ScramblingMode::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        4,
        "ScramblingMode sweep produced {} variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "ScramblingMode drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: Polarization ────────────────────────────────────────────────────────

#[test]
fn polarization_toml_matches_enum() {
    let toml = include_str!("../spec_tables/polarization.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // 2-bit exhaustive: from_u8 masks to 2 bits, all 4 values are named.
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=3 {
        let v = Polarization::from_u8(b);
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        4,
        "Polarization sweep produced {} variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "Polarization drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: FecOuter ────────────────────────────────────────────────────────────

#[test]
fn fec_outer_toml_matches_enum() {
    let toml = include_str!("../spec_tables/fec_outer.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // FecOuter has no public from_u8; enumerate named variants directly.
    let named: &[(u8, FecOuter)] = &[
        (0x00, FecOuter::NotDefined),
        (0x01, FecOuter::NoOuterFec),
        (0x02, FecOuter::ReedSolomon204_188),
    ];
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for (wire, v) in named {
        code_set.insert((*wire, format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        3,
        "FecOuter has {} named variants, expected 3",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "FecOuter drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: TsGsMode ────────────────────────────────────────────────────────────

#[test]
fn ts_gs_mode_toml_matches_enum() {
    let toml = include_str!("../spec_tables/ts_gs_mode.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = TsGsMode::from_u8(b);
        if matches!(v, TsGsMode::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        3,
        "TsGsMode sweep produced {} variants, expected 3",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "TsGsMode drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: S2XMode ─────────────────────────────────────────────────────────────

#[test]
fn s2x_mode_toml_matches_enum() {
    let toml = include_str!("../spec_tables/s2x_mode.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = S2XMode::from_u8(b);
        if matches!(v, S2XMode::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        4,
        "S2XMode sweep produced {} variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "S2XMode drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: T2SisoMiso ──────────────────────────────────────────────────────────

#[test]
fn t2_siso_miso_toml_matches_enum() {
    let toml = include_str!("../spec_tables/t2_siso_miso.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = T2SisoMiso::from_u8(b);
        if matches!(v, T2SisoMiso::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        2,
        "T2SisoMiso sweep produced {} variants, expected 2",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "T2SisoMiso drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: ShDiversityMode ─────────────────────────────────────────────────────

#[test]
fn sh_diversity_mode_toml_matches_enum() {
    let toml = include_str!("../spec_tables/sh_diversity_mode.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = ShDiversityMode::from_u8(b);
        if matches!(v, ShDiversityMode::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        5,
        "ShDiversityMode sweep produced {} variants, expected 5",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "ShDiversityMode drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: C2TuningFrequencyType ───────────────────────────────────────────────

#[test]
fn c2_tuning_frequency_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/c2_tuning_frequency_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = C2TuningFrequencyType::from_u8(b);
        if matches!(v, C2TuningFrequencyType::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        3,
        "C2TuningFrequencyType sweep produced {} variants, expected 3",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "C2TuningFrequencyType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: UriLinkageType ──────────────────────────────────────────────────────

#[test]
fn uri_linkage_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/uri_linkage_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = UriLinkageType::from_u8(b);
        if matches!(v, UriLinkageType::Other(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        4,
        "UriLinkageType sweep produced {} variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "UriLinkageType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: ExtensionTag ────────────────────────────────────────────────────────

#[test]
fn extension_tag_toml_matches_enum() {
    let toml = include_str!("../spec_tables/extension_tag.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // ExtensionTag is #[repr(u8)] with no from_u8; enumerate all variants directly.
    use ExtensionTag::*;
    let all_variants: &[ExtensionTag] = &[
        ImageIcon,
        T2DeliverySystem,
        ShDeliverySystem,
        SupplementaryAudio,
        NetworkChangeNotify,
        Message,
        TargetRegion,
        TargetRegionName,
        ServiceRelocated,
        C2DeliverySystem,
        VideoDepthRange,
        T2mi,
        UriLinkage,
        Ac4,
        C2BundleDeliverySystem,
        S2XSatelliteDeliverySystem,
        AudioPreselection,
        TtmlSubtitling,
        ServiceProminence,
        VvcSubpictures,
        S2Xv2SatelliteDeliverySystem,
    ];
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for v in all_variants {
        code_set.insert((*v as u8, format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        21,
        "ExtensionTag has {} variants, expected 21",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "ExtensionTag drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: RunningStatus ───────────────────────────────────────────────────────

#[test]
fn running_status_toml_matches_enum() {
    let toml = include_str!("../spec_tables/running_status.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // from_u8 applies & 0x07; sweep 0..=7 to get canonical values.
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=7 {
        let v = RunningStatus::from_u8(b);
        if matches!(v, RunningStatus::Reserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        6,
        "RunningStatus sweep produced {} variants, expected 6",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "RunningStatus drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: IntActionType ───────────────────────────────────────────────────────

#[test]
fn int_action_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/int_action_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = IntActionType::from_u8(b);
        if matches!(v, IntActionType::DvbReserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        2,
        "IntActionType sweep produced {} variants, expected 2",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "IntActionType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: UntActionType ───────────────────────────────────────────────────────

#[test]
fn unt_action_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/unt_action_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = UntActionType::from_u8(b);
        if matches!(
            v,
            UntActionType::DvbReserved(_) | UntActionType::UserDefined(_)
        ) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        2,
        "UntActionType sweep produced {} variants, expected 2",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "UntActionType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: ControlCode ─────────────────────────────────────────────────────────

#[test]
fn control_code_toml_matches_enum() {
    let toml = include_str!("../spec_tables/control_code.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = ControlCode::from_u8(b);
        if matches!(v, ControlCode::Unallocated(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        9,
        "ControlCode sweep produced {} variants, expected 9",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "ControlCode drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: ReferenceType ───────────────────────────────────────────────────────

#[test]
fn reference_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/reference_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // from_u8 applies & 0x0F; sweep 0..=15 for canonical values.
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=15 {
        let v = ReferenceType::from_u8(b);
        if matches!(v, ReferenceType::Unallocated(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        3,
        "ReferenceType sweep produced {} variants, expected 3",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "ReferenceType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: CridAuthorityPolicy ─────────────────────────────────────────────────

#[test]
fn crid_authority_policy_toml_matches_enum() {
    let toml = include_str!("../spec_tables/crid_authority_policy.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // Exhaustive 2-bit field: from_u8 applies & 0x03; all 4 values named.
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=3 {
        let v = CridAuthorityPolicy::from_u8(b);
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        4,
        "CridAuthorityPolicy sweep produced {} variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "CridAuthorityPolicy drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: LinkType ────────────────────────────────────────────────────────────

#[test]
fn link_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/link_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // from_u8 applies & 0x0F; sweep 0..=15.
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=15 {
        let v = LinkType::from_u8(b);
        if matches!(v, LinkType::DvbReserved(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        4,
        "LinkType sweep produced {} variants, expected 4",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "LinkType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: DescriptorType ──────────────────────────────────────────────────────

#[test]
fn descriptor_type_toml_matches_enum() {
    let toml = include_str!("../spec_tables/descriptor_type.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    // Exclude all catch-all variants (IsoReserved, DvbReserved, UserDefined).
    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = DescriptorType::from_u8(b);
        if matches!(
            v,
            DescriptorType::IsoReserved(_)
                | DescriptorType::DvbReserved(_)
                | DescriptorType::UserDefined(_)
        ) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        3,
        "DescriptorType sweep produced {} variants, expected 3",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "DescriptorType drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}

// ── test: TvaRunningStatus ────────────────────────────────────────────────────

#[test]
fn tva_running_status_toml_matches_enum() {
    let toml = include_str!("../spec_tables/tva_running_status.toml");
    let entries = parse_entries(toml);
    let toml_set: BTreeSet<(u8, String)> = entries
        .iter()
        .map(|(v, var, _)| (*v, var.clone()))
        .collect();

    let mut code_set: BTreeSet<(u8, String)> = BTreeSet::new();
    for b in 0u8..=255 {
        let v = TvaRunningStatus::from_u8(b);
        if matches!(v, TvaRunningStatus::Unallocated(_)) {
            continue;
        }
        code_set.insert((v.to_u8(), format!("{v:?}")));
    }

    assert_eq!(
        code_set.len(),
        7,
        "TvaRunningStatus sweep produced {} variants, expected 7",
        code_set.len()
    );

    let only_in_toml: BTreeSet<_> = toml_set.difference(&code_set).collect();
    let only_in_code: BTreeSet<_> = code_set.difference(&toml_set).collect();
    assert!(
        only_in_toml.is_empty() && only_in_code.is_empty(),
        "TvaRunningStatus drift detected!\n  only in TOML: {only_in_toml:?}\n  only in code: {only_in_code:?}"
    );
}
