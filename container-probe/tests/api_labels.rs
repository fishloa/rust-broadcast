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

/// Finding 9: the FLV, MXF and MPEG-PS probers now report what they already
/// decoded, rather than `Detail::None` — a caller must not have to re-walk the
/// header to recover flags the prober read.
#[test]
fn flv_mxf_mpegps_report_detail() {
    // FLV: TypeFlags + DataOffset.
    let flv = container_probe::probe(&fixture("fixtures/flv/av.flv"));
    match flv {
        Probe::Identified { detail, .. } => match detail {
            Detail::Flv {
                has_audio,
                has_video,
                data_offset,
                ..
            } => {
                assert!(has_video, "av.flv carries video tags");
                assert!(has_audio, "av.flv carries audio tags");
                assert!(data_offset >= 9);
            }
            other => panic!("FLV must report Detail::Flv, got {other:?}"),
        },
        other => panic!("av.flv must identify, got {other:?}"),
    }

    // MXF: PartitionKind byte.
    let mxf = container_probe::probe(&fixture("fixtures/mxf/op1a_mpeg2_pcm.mxf"));
    match mxf {
        Probe::Identified { detail, .. } => match detail {
            Detail::Mxf { partition_kind, .. } => {
                assert!(
                    (0x02..=0x04).contains(&partition_kind),
                    "partition kind {partition_kind:#x} must be Header/Body/Footer"
                );
            }
            other => panic!("MXF must report Detail::Mxf, got {other:?}"),
        },
        other => panic!("mxf must identify, got {other:?}"),
    }

    // MPEG-PS: structural marker bits validated.
    let ps = container_probe::probe(&fixture("fixtures/ps/h264_ac3.ps"));
    match ps {
        Probe::Identified { detail, .. } => match detail {
            Detail::MpegPs {
                structurally_valid, ..
            } => assert!(structurally_valid, "h264_ac3.ps markers validate"),
            other => panic!("MPEG-PS must report Detail::MpegPs, got {other:?}"),
        },
        other => panic!("h264_ac3.ps must identify, got {other:?}"),
    }
}
