//! `Role` (ISO/IEC 23009-1 5.8.5.5 Table 34) token table and the descriptor
//! helpers, plus the error text of the `xs:duration` reader.

use broadcast_dash::descriptors::{Role, find};
use broadcast_dash::duration::DashParseError;
use broadcast_dash::model::Descriptor;

/// Every Table 34 token with the variant it must classify to.
const TABLE_34: &[(&str, Role)] = &[
    ("caption", Role::Caption),
    ("subtitle", Role::Subtitle),
    ("main", Role::Main),
    ("alternate", Role::Alternate),
    ("supplementary", Role::Supplementary),
    ("commentary", Role::Commentary),
    ("dub", Role::Dub),
    ("description", Role::Description),
    ("sign", Role::Sign),
    ("metadata", Role::Metadata),
    (
        "enhanced-audio-intelligibility",
        Role::EnhancedAudioIntelligibility,
    ),
    ("emergency", Role::Emergency),
    ("forced-subtitle", Role::ForcedSubtitle),
    ("easyreader", Role::EasyReader),
    ("karaoke", Role::Karaoke),
];

#[test]
fn every_table_34_token_classifies_to_its_role() {
    assert_eq!(TABLE_34.len(), 15);
    for (token, role) in TABLE_34 {
        assert_eq!(Role::from_value(token), *role, "{token}");
    }
    assert_eq!(Role::from_value("other"), Role::Other);
    assert_eq!(Role::from_value(""), Role::Other);
    assert_eq!(Role::from_value("Main"), Role::Other);
    assert_eq!(Role::from_value("main "), Role::Other);
}

#[test]
fn every_role_names_the_token_it_classifies() {
    for (token, role) in TABLE_34 {
        assert_eq!(role.name(), *token);
        assert_eq!(role.to_string(), *token);
        assert_eq!(Role::from_value(role.name()), *role);
    }
    assert_eq!(Role::Other.name(), "other");
    assert_eq!(Role::Other.to_string(), "other");
}

fn descriptor(scheme: &str) -> Descriptor {
    let mut d = Descriptor::default();
    d.scheme_id_uri = scheme.to_string();
    d
}

#[test]
fn find_returns_the_matching_descriptor() {
    let role = descriptor("urn:mpeg:dash:role:2011");
    let accessibility = descriptor("urn:mpeg:dash:accessibility:2011");
    let descriptors = [role.clone(), accessibility.clone()];
    assert_eq!(find(&descriptors, "urn:mpeg:dash:role:2011"), Some(&role));
    assert_eq!(
        find(&descriptors, "urn:mpeg:dash:accessibility:2011"),
        Some(&accessibility)
    );
    assert_eq!(find(&descriptors, "urn:absent"), None);
    assert_eq!(find(&[], "urn:mpeg:dash:role:2011"), None);
}

#[test]
fn find_returns_the_first_of_two_equal_schemes() {
    let mut first = descriptor("urn:mpeg:dash:role:2011");
    first.value = Some("main".to_string());
    let mut second = descriptor("urn:mpeg:dash:role:2011");
    second.value = Some("alternate".to_string());
    let descriptors = [first.clone(), second];
    assert_eq!(
        find(&descriptors, "urn:mpeg:dash:role:2011").map(|d| d.value.as_deref()),
        Some(Some("main"))
    );
}

#[test]
fn an_invalid_duration_names_the_offending_value() {
    let error = DashParseError::InvalidDuration {
        value: "PT1X".to_string(),
    };
    assert_eq!(error.to_string(), "invalid xs:duration \"PT1X\"");
}
