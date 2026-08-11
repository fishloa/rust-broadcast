//! WP2 structural prober fixtures — exact verdicts (never a disjunction).
//!
//! Each of these asserts ONE exact `Probe` out of `container_probe`, matching
//! full field values. Fixture paths are workspace-relative; we join
//! `CARGO_MANIFEST_DIR` to `../<path>` as `fixture_ts.rs` does.

use container_probe::{Confidence, Detail::Ebml, Detail::Isobmff, DocType, Format, Probe};
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
                }
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
                    doc_type: DocType::Matroska
                }
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
                    doc_type: DocType::Webm
                }
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
