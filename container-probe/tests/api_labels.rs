//! Public API label + accessor tests (audit finding 9).
//!
//! - `Confidence` now has `name()` and `Display`, so a caller never has to
//!   re-implement the six-tier table by hand (the examples once hardcoded the
//!   magic numbers).
//! - `Detail`'s `Display` is lossless: `Detail::Ts { stride, phase }` renders
//!   its fields instead of collapsing to `"Ts"`.
//! - `major_brand_str()` exposes the registered ISOBMFF brand as a string.

use container_probe::{Confidence, Detail, Probe};
use std::fmt::Write;
use std::fs;

fn fixture(rel: &str) -> Vec<u8> {
    fs::read(format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
        .unwrap_or_else(|e| panic!("failed to read fixture {rel}: {e}"))
}

/// `Confidence` exposes every tier by name and Display, in one place — a caller
/// never hardcodes the score table.
#[test]
fn confidence_has_named_tiers() {
    let tiers = [
        (Confidence::CERTAIN, "CERTAIN"),
        (Confidence::STRONG, "STRONG"),
        (Confidence::STRUCTURAL, "STRUCTURAL"),
        (Confidence::LATTICE_STRONG, "LATTICE_STRONG"),
        (Confidence::LATTICE_WEAK, "LATTICE_WEAK"),
        (Confidence::HEURISTIC, "HEURISTIC"),
    ];
    for (c, name) in tiers {
        assert_eq!(c.name(), name);
        assert_eq!(c.to_string(), name, "Display must equal name()");
    }
}

/// `Detail`'s `Display` is lossless: it renders a `Ts` lattice's stride and
/// phase rather than collapsing to just `"Ts"`. The `Detail::Ts` variant is
/// `#[non_exhaustive]`, so it is obtained from a real probe result rather than
/// constructed.
#[test]
fn detail_display_is_lossless() {
    let data = fixture("fixtures/container-probe/m2ts_192.m2ts");
    let detail = match container_probe::probe(&data) {
        Probe::Identified { detail, .. } => detail,
        other => panic!("m2ts_192.m2ts must identify, got {other:?}"),
    };
    let rendered = detail.to_string();
    assert!(
        rendered.contains("Ts") && rendered.contains("192") && rendered.contains("4"),
        "Detail::Ts Display dropped data: {rendered:?}"
    );
    assert!(
        rendered != "Ts",
        "Detail::Ts must not collapse to its name only"
    );
}

/// `Identified` objects expose the ISOBMFF major brand as a string.
#[test]
fn major_brand_str_returns_the_brand() {
    let data = fixture("fixtures/mp4/h264_high.mp4");
    match container_probe::probe(&data) {
        Probe::Identified { detail, .. } => {
            assert_eq!(detail.major_brand_str(), Some("isom"));
        }
        other => panic!("h264_high.mp4 must identify, got {other:?}"),
    }
}

/// The accessor returns `None` for non-ISOBMFF results (here a TS lattice).
#[test]
fn major_brand_str_is_none_for_ts() {
    let data = fixture("fixtures/ts/h264_aac.ts");
    match container_probe::probe(&data) {
        Probe::Identified { detail, .. } => {
            assert_eq!(detail.major_brand_str(), None);
        }
        other => panic!("h264_aac.ts must identify, got {other:?}"),
    }
}

/// `Detail` still delegates to `name()` for naming (issue #204 convention).
#[test]
fn detail_names() {
    assert_eq!(Detail::None.name(), "None");
    let mut s = String::new();
    write!(&mut s, "{}", Detail::None).expect("fmt");
    assert_eq!(s, "None");
}
