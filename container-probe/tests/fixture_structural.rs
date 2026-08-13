//! WP2 structural prober fixtures — exact verdicts (never a disjunction).
//!
//! Each of these asserts ONE exact `Probe` out of `container_probe`, matching
//! full field values. Fixture paths are workspace-relative; we join
//! `CARGO_MANIFEST_DIR` to `../<path>` as `fixture_ts.rs` does.

use container_probe::{
    Confidence, Detail::Ebml, Detail::Isobmff, DocType, Format, IsobmffLayout, Probe,
};
use std::fs;

/// Read a workspace-relative fixture.
fn fixture(rel: &str) -> Vec<u8> {
    fs::read(format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
        .unwrap_or_else(|e| panic!("failed to read fixture {rel}: {e}"))
}

/// Assert an ISOBMFF file with the given brand. `boxes_walked` is measured and
/// not pinned here, so the detail is matched structurally; everything else is
/// exact.
fn assert_isobmff(rel: &str, brand: &[u8; 4]) {
    let p = container_probe::probe(&fixture(rel));
    assert!(
        matches!(
            p,
            Probe::Identified {
                format: Format::Isobmff,
                confidence: Confidence::STRUCTURAL,
                detail: Isobmff {
                    major_brand: Some(b),
                    ..
                },
                ..
            } if &b == brand
        ),
        "{rel}: expected Isobmff STRUCTURAL brand={:?}, got {p:?}",
        String::from_utf8_lossy(brand)
    );
}

/// Assert a Matroska file (`DocType == "matroska"`).
fn assert_matroska(rel: &str) {
    let p = container_probe::probe(&fixture(rel));
    assert!(
        matches!(
            p,
            Probe::Identified {
                format: Format::Matroska,
                confidence: Confidence::CERTAIN,
                detail: Ebml {
                    doc_type: DocType::Matroska,
                },
                ..
            }
        ),
        "{rel}: expected Matroska CERTAIN DocType::Matroska, got {p:?}"
    );
}

/// Assert a WebM file (`DocType == "webm"`).
fn assert_webm(rel: &str) {
    let p = container_probe::probe(&fixture(rel));
    assert!(
        matches!(
            p,
            Probe::Identified {
                format: Format::WebM,
                confidence: Confidence::CERTAIN,
                detail: Ebml {
                    doc_type: DocType::Webm,
                },
                ..
            }
        ),
        "{rel}: expected WebM CERTAIN DocType::Webm, got {p:?}"
    );
}

/// Assert an MXF file (`CERTAIN`)
fn assert_mxf(rel: &str) {
    let p = container_probe::probe(&fixture(rel));
    assert!(
        matches!(
            p,
            Probe::Identified {
                format: Format::Mxf,
                confidence: Confidence::CERTAIN,
                ..
            }
        ),
        "{rel}: expected Mxf CERTAIN, got {p:?}"
    );
}

// ---------------------------------------------------------------------------
// ISOBMFF.
// ---------------------------------------------------------------------------
//
// Brands below are measured from each leading box's bytes 8..12 (the `ftyp`
// major brand), not taken from the brief — the brief itself says to measure.
// Most are `isom`; the fragmented `frag/*.frag.mp4` files are `iso5`.

#[test]
fn isobmff_brand_isom() {
    // # Mutation proof
    //
    // Breaking the box-chain threshold (changing `boxes >= 2` to `boxes >= 99`
    // in `isobmff::probe`) makes every MP4 here probe to `Unknown`:
    // ```
    // fixtures/mp4/h264_high.mp4: expected Isobmff STRUCTURAL brand="isom",
    //     got Unknown
    // ```
    // The count-and-chain walk is the prober's real structural check.
    for f in [
        "fixtures/mp4/h264_high.mp4",
        "fixtures/mp4/hevc_main.mp4",
        "fixtures/mp4/av1.mp4",
        "fixtures/mp4/vp9.mp4",
        "fixtures/mp4/opus.mp4",
        "fixtures/mp4/flac.mp4",
        "fixtures/mp4/cenc.mp4",
        "fixtures/mp4/stpp.mp4",
        "fixtures/mp4/prft.mp4",
        "fixtures/mp4/colr_hdr.mp4",
        "fixtures/mp4/aac_sgpd.mp4",
        "fixtures/mp4/cmaf/av_frag.mp4",
        "fixtures/mp4/progressive/av_prog.mp4",
    ] {
        assert_isobmff(f, b"isom");
    }
}

#[test]
fn isobmff_brand_iso5_fragmented() {
    // Fragmented init/segment MP4s whose `ftyp` major brand is `iso5`.
    for f in [
        "fixtures/mp4/frag/av1.frag.mp4",
        "fixtures/mp4/frag/vvc.frag.mp4",
    ] {
        assert_isobmff(f, b"iso5");
    }
}

// ---------------------------------------------------------------------------
// Matroska.
// ---------------------------------------------------------------------------

#[test]
fn matroska_mkv() {
    // # Mutation proof
    //
    // Disabling the DocType element read (changing `id == ID_DOC_TYPE` to a
    // constant `false` in `ebml::find_doc_type`) collapses these to the
    // magic-only STRONG verdict rather than CERTAIN-with-a-DocType:
    // ```
    // fixtures/mkv/h264_aac.mkv: expected Matroska CERTAIN DocType::Matroska,
    //     got Identified { format: Matroska, confidence: Confidence(192),
    //                      detail: Ebml { doc_type: Other } }
    // ```
    for f in [
        "fixtures/mkv/h264_aac.mkv",
        "fixtures/mkv/hevc_aac.mkv",
        "fixtures/mkv/vp9_opus.mkv",
    ] {
        assert_matroska(f);
    }
}

// ---------------------------------------------------------------------------
// WebM.
// ---------------------------------------------------------------------------

#[test]
fn webm() {
    // Same EBML mutation as `matroska_mkv`: disabling the DocType lookup makes
    // a real "webm" probe to `Matroska` + `DocType::Other` at confidence 192
    // (STRONG, magic-only) instead of `WebM`/`Webm` at 240:
    // ```
    // fixtures/webm/vorbis.webm: expected WebM CERTAIN DocType::Webm, got
    //     Identified { format: Matroska, confidence: Confidence(192),
    //                  detail: Ebml { doc_type: Other } }
    // ```
    for f in [
        "fixtures/webm/vorbis.webm",
        "fixtures/webm/vp8_opus.webm",
        "fixtures/webm/vp9_opus.webm",
    ] {
        assert_webm(f);
    }
}

// ---------------------------------------------------------------------------
// MXF.
// ---------------------------------------------------------------------------

#[test]
fn mxf() {
    // # Mutation proof
    //
    // Forcing `ber_length_well_formed` to always return `false` drops MXF from
    // CERTAIN to the magic-only STRONG, failing this test:
    // ```
    // fixtures/mxf/op1a_mpeg2_pcm.mxf: expected Mxf CERTAIN, got
    //     Identified { format: Mxf, confidence: Confidence(192), detail: None }
    // ```
    assert_mxf("fixtures/mxf/op1a_mpeg2_pcm.mxf");
    assert_mxf("st377-1/tests/fixtures/synthetic_minimal.mxf");
}

// ---------------------------------------------------------------------------
// MPEG-PS.
// ---------------------------------------------------------------------------

/// The measured verdict for the single MPEG-PS fixture: the pack header's
/// marker bits validate, so it is `STRUCTURAL`.
///
/// # Mutation proof
///
/// Forcing `markers_valid` to always return `false` drops MPEG-PS to the bare
/// start-code HEURISTIC, failing this test:
/// ```
/// fixtures/ps/h264_ac3.ps: expected MpegPs STRUCTURAL, got
///     Identified { format: MpegPs, confidence: Confidence(64), detail: None }
/// ```
#[test]
fn mpeg_ps() {
    let p = container_probe::probe(&fixture("fixtures/ps/h264_ac3.ps"));
    assert!(
        matches!(
            p,
            Probe::Identified {
                format: Format::MpegPs,
                confidence: Confidence::STRUCTURAL,
                ..
            }
        ),
        "fixtures/ps/h264_ac3.ps: expected MpegPs STRUCTURAL, got {p:?}"
    );
}

// --- ISOBMFF layout: fragmented vs progressive (issue #960 follow-up) ---

/// Assert an ISOBMFF fixture reports the layout it actually has.
fn assert_layout(rel: &str, expect: IsobmffLayout) {
    let data = fixture(rel);
    match container_probe::probe(&data) {
        Probe::Identified {
            format: Format::Isobmff,
            detail: Isobmff { layout, .. },
            ..
        } => assert_eq!(layout, expect, "{rel}"),
        other => panic!("{rel} must be Isobmff, got {other:?}"),
    }
}

/// Progressive files — sample tables in `moov`, no `moof`. These are what
/// `transmux::ProgressiveDemux` handles.
#[test]
fn progressive_mp4s_report_progressive_layout() {
    for rel in [
        "fixtures/mp4/h264_high.mp4",
        "fixtures/mp4/cenc.mp4",
        "fixtures/mp4/av1.mp4",
        "fixtures/mp4/progressive/av_prog.mp4",
        "fixtures/mp4/progressive/av_faststart.mp4",
    ] {
        assert_layout(rel, IsobmffLayout::Progressive);
    }
}

/// Fragmented files — `moof` movie fragments. These are what
/// `transmux::Fmp4Demux` handles.
///
/// `cmaf/av_frag.mp4` is the fixture that proves the walk is necessary: its
/// major brand is `isom`, exactly the same brand every progressive file above
/// carries, yet it is fragmented. **The brand cannot discriminate** — only
/// seeing a `moof` box can.
#[test]
fn fragmented_mp4s_report_fragmented_layout() {
    for rel in [
        "fixtures/mp4/frag/av1.frag.mp4",
        "fixtures/mp4/frag/vvc.frag.mp4",
        "fixtures/mp4/cmaf/av_frag.mp4",
        "fixtures/mp4/llcmaf/ll_chunked.mp4",
    ] {
        assert_layout(rel, IsobmffLayout::Fragmented);
    }
}

/// The brand really is ambiguous: at least one fragmented and one progressive
/// fixture share the identical `isom` major brand. Pinned so nobody later
/// "optimises" the box walk away in favour of a brand lookup.
#[test]
fn major_brand_alone_cannot_discriminate_layout() {
    let brand_of = |rel: &str| -> Option<[u8; 4]> {
        match container_probe::probe(&fixture(rel)) {
            Probe::Identified {
                detail: Isobmff { major_brand, .. },
                ..
            } => major_brand,
            other => panic!("{rel}: {other:?}"),
        }
    };
    let progressive = brand_of("fixtures/mp4/h264_high.mp4");
    let fragmented = brand_of("fixtures/mp4/cmaf/av_frag.mp4");
    assert_eq!(
        progressive, fragmented,
        "these two fixtures must share a brand for this test to mean anything"
    );
    assert_eq!(progressive, Some(*b"isom"));
}

/// A truncated prefix reports `Unknown`, never `Progressive`.
///
/// This is the regression guard for a real trap. Every fragmented file OPENS
/// with a `ftyp` + `moov` init segment and only reaches its first `moof`
/// later, so a short prefix of a fragmented file is byte-for-byte
/// indistinguishable from a progressive one. An earlier revision concluded
/// `saw_moov && !saw_moof => Progressive` and therefore called the first 64
/// bytes of this genuinely fragmented CMAF file `Progressive` — which would
/// have sent a consumer to `ProgressiveDemux` for an fMP4 file.
///
/// `Progressive` is now claimed only when the walk consumed the entire
/// supplied buffer (and the buffer was not clipped by the probe budget), so
/// the absence of a `moof` actually means something.
///
/// MUTATION VERIFIED: dropping the `walked_whole_input` condition turns this
/// red with `left: Progressive, right: Unknown`.
#[test]
fn a_prefix_without_moof_or_moov_reports_unknown_layout() {
    let data = fixture("fixtures/mp4/cmaf/av_frag.mp4");
    // 64 bytes covers `ftyp` and lands inside the following box, well before
    // any `moof`.
    match container_probe::probe(&data[..64]) {
        Probe::Identified {
            format: Format::Isobmff,
            detail: Isobmff { layout, .. },
            ..
        } => assert_eq!(layout, IsobmffLayout::Unknown),
        other => panic!("a 64-byte ISOBMFF prefix should still identify, got {other:?}"),
    }
}
