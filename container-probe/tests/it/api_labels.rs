//! Public API label + accessor tests (audit finding 9).
//!
//! - `Confidence` now has `name()` and `Display`, so a caller never has to
//!   re-implement the six-tier table by hand (the examples once hardcoded the
//!   magic numbers).
//! - `Detail`'s `Display` is lossless: `Detail::Ts { stride, phase }` renders
//!   its fields instead of collapsing to `"Ts"`.
//! - `major_brand_str()` exposes the registered ISOBMFF brand as a string.

use container_probe::{Confidence, Detail, DocType, Format, IsobmffLayout, PartitionKind, Probe};
use std::fmt::Write;

fn fixture(rel: &str) -> Vec<u8> {
    test_fixtures::read(rel.trim_start_matches("fixtures/"))
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
    let data = fixture("fixtures/ts/m2ts-192.m2ts");
    let detail = match container_probe::probe(&data) {
        Probe::Identified { detail, .. } => detail,
        other => panic!("m2ts-192.m2ts must identify, got {other:?}"),
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
    let data = fixture("fixtures/mp4/h264-high.mp4");
    match container_probe::probe(&data) {
        Probe::Identified { detail, .. } => {
            assert_eq!(detail.major_brand_str(), Some("isom"));
        }
        other => panic!("h264-high.mp4 must identify, got {other:?}"),
    }
}

/// The accessor returns `None` for non-ISOBMFF results (here a TS lattice).
#[test]
fn major_brand_str_is_none_for_ts() {
    let data = fixture("fixtures/ts/h264-aac.ts");
    match container_probe::probe(&data) {
        Probe::Identified { detail, .. } => {
            assert_eq!(detail.major_brand_str(), None);
        }
        other => panic!("h264-aac.ts must identify, got {other:?}"),
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
    let mxf = container_probe::probe(&fixture("fixtures/mxf/op1a-mpeg2-pcm.mxf"));
    match mxf {
        Probe::Identified { detail, .. } => match detail {
            Detail::Mxf { partition_kind, .. } => {
                // Typed now: assert the decoded kind, not a raw byte range.
                // The previous `(0x02..=0x04).contains(&partition_kind)` is the
                // lookup every consumer had to re-implement — the exact thing
                // the decode-completeness rule forbids.
                assert!(
                    matches!(
                        partition_kind,
                        PartitionKind::Header | PartitionKind::Body | PartitionKind::Footer
                    ),
                    "a real MXF fixture must decode to a named partition kind, got \
                     {partition_kind:?}"
                );
                assert_eq!(partition_kind.name(), "Header");
                assert_eq!(partition_kind.to_string(), "Header");
            }
            other => panic!("MXF must report Detail::Mxf, got {other:?}"),
        },
        other => panic!("mxf must identify, got {other:?}"),
    }

    // MPEG-PS: structural marker bits validated.
    let ps = container_probe::probe(&fixture("fixtures/ps/h264-ac3.ps"));
    match ps {
        Probe::Identified { detail, .. } => match detail {
            Detail::MpegPs {
                structurally_valid, ..
            } => assert!(structurally_valid, "h264-ac3.ps markers validate"),
            other => panic!("MPEG-PS must report Detail::MpegPs, got {other:?}"),
        },
        other => panic!("h264-ac3.ps must identify, got {other:?}"),
    }
}

/// `PartitionKind::as_u8` is the stable wire byte (SMPTE ST 377-1 §7.2-7.4).
#[test]
fn partition_kind_wire_bytes_are_documented() {
    assert_eq!(PartitionKind::Header.as_u8(), 0x02);
    assert_eq!(PartitionKind::Body.as_u8(), 0x03);
    assert_eq!(PartitionKind::Footer.as_u8(), 0x04);

    // `Other` carries a value outside 0x02..=0x04 and is read from a real
    // probe, since the variant is `#[non_exhaustive]` to downstream code.
    let mut key = [0u8; 20];
    key[..7].copy_from_slice(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01]);
    key[13] = 0x42;
    let other = match container_probe::probe(&key) {
        Probe::Identified {
            detail: Detail::Mxf { partition_kind, .. },
            ..
        } => partition_kind,
        other => panic!("a key-prefixed buffer must report an MXF detail, got {other:?}"),
    };
    assert_eq!(other.as_u8(), 0x42);
    assert_eq!(other.to_string(), "reserved(0x42)");
}

/// Every format's documented label, and `Display` parity (#204).
#[test]
fn every_format_has_its_documented_name() {
    let named = [
        (Format::MpegTs, "MpegTs"),
        (Format::Isobmff, "Isobmff"),
        (Format::MpegPs, "MpegPs"),
        (Format::Matroska, "Matroska"),
        (Format::WebM, "WebM"),
        (Format::Flv, "Flv"),
        (Format::Mxf, "Mxf"),
        (Format::Wav, "Wav"),
        (Format::Ogg, "Ogg"),
        (Format::Asf, "Asf"),
        (Format::AdtsAac, "AdtsAac"),
        (Format::Mp3, "Mp3"),
        (Format::AnnexB, "AnnexB"),
    ];
    for (format, name) in named {
        assert_eq!(format.name(), name);
        assert_eq!(format.to_string(), name, "Display must equal name()");
    }
}

/// The ISOBMFF layout labels used by `Detail::Isobmff`.
#[test]
fn every_isobmff_layout_has_its_documented_name() {
    for (layout, name) in [
        (IsobmffLayout::Fragmented, "fragmented"),
        (IsobmffLayout::Progressive, "progressive"),
        (IsobmffLayout::Unknown, "unknown"),
    ] {
        assert_eq!(layout.name(), name);
        assert_eq!(layout.to_string(), name, "Display must equal name()");
    }
}

/// The EBML `DocType` labels.
#[test]
fn every_doc_type_has_its_documented_name() {
    for (doc_type, name) in [
        (DocType::Webm, "Webm"),
        (DocType::Matroska, "Matroska"),
        (DocType::Other, "Other"),
    ] {
        assert_eq!(doc_type.name(), name);
        assert_eq!(doc_type.to_string(), name, "Display must equal name()");
    }
}

/// `Probe::name` labels each conclusion, and `best_confidence` reports the
/// winning tier for a match and `None` for an undecided/unknown probe.
#[test]
fn probe_names_and_best_confidence() {
    let ts = container_probe::probe(&fixture("fixtures/ts/h264-aac.ts"));
    assert_eq!(ts.name(), "Identified");
    assert_eq!(ts.best_confidence(), Some(Confidence::LATTICE_STRONG));
    assert_eq!(ts.to_string(), ts.name());

    let mp4 = container_probe::probe(&fixture("fixtures/mp4/h264-high.mp4"));
    assert_eq!(mp4.name(), "Identified");
    assert_eq!(mp4.best_confidence(), Some(Confidence::STRUCTURAL));

    let mkv = container_probe::probe(&fixture("fixtures/mkv/h264-aac.mkv"));
    assert_eq!(mkv.name(), "Identified");
    assert_eq!(mkv.best_confidence(), Some(Confidence::CERTAIN));

    let short = container_probe::probe(&[]);
    assert_eq!(short.name(), "Insufficient");
    assert_eq!(short.best_confidence(), None);
    assert_eq!(short.to_string(), short.name());

    let unknown = container_probe::probe(&[0u8; 4096]);
    assert_eq!(unknown.name(), "Unknown");
    assert_eq!(unknown.best_confidence(), None);
    assert_eq!(unknown.to_string(), unknown.name());
}
