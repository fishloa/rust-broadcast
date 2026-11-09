//! `clap` over-rides the codec's own crop in the demuxed display geometry
//! (ISO/IEC 14496-12:2026 §12.1.4.1), with the signed 32-bit fields and the
//! centre/offset arithmetic of §12.1.4.2.
//!
//! `fixtures/mp4/clap-vs-crop.mp4` (GPAC, see `fixtures/mp4/PROVENANCE.md`) is a
//! 1920x1080 H.264 entry whose SPS crops 1088 rows to 1080 and whose `clap` is a
//! 1904x1072 aperture centred in the entry. The other cases patch that box's
//! eight fields in place.
//!
//! Spec input: `private/specs-verified/mpeg/isobmff/sample-entry-ext.md`.

use broadcast_common::Unpackage;
use transmux::ir::{CodecConfig, DisplayGeometry, PictureRect};
use transmux::progressive_demux::ProgressiveDemux;

const FIXTURE: &str = "mp4/clap-vs-crop.mp4";
const CLAP_BODY_LEN: usize = 32;

const CODED: (u32, u32) = (1920, 1088);
const SPS_CROP: (u32, u32, u32, u32) = (0, 0, 1920, 1080);

type Frac = (i32, i32);

fn display_of(bytes: &[u8]) -> DisplayGeometry {
    let media = ProgressiveDemux::new(bytes.len())
        .expect("demux")
        .unpackage(bytes)
        .expect("progressive demux");
    let track = media
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("an AVC track");
    track.spec.display.expect("display geometry")
}

fn with_clap(width: Frac, height: Frac, horiz: Frac, vert: Frac) -> Vec<u8> {
    let mut bytes = test_fixtures::read(FIXTURE);
    let at = bytes
        .windows(4)
        .position(|w| w == b"clap")
        .expect("clap box")
        + 4;
    for (i, (n, d)) in [width, height, horiz, vert].into_iter().enumerate() {
        bytes[at + i * 8..at + i * 8 + 4].copy_from_slice(&n.to_be_bytes());
        bytes[at + i * 8 + 4..at + i * 8 + 8].copy_from_slice(&d.to_be_bytes());
    }
    assert!(bytes.len() > at + CLAP_BODY_LEN);
    bytes
}

fn sps_crop() -> PictureRect {
    rect(SPS_CROP.0, SPS_CROP.1, SPS_CROP.2, SPS_CROP.3)
}

fn rect(left: u32, top: u32, width: u32, height: u32) -> PictureRect {
    PictureRect::new(left, top, width, height)
}

#[test]
fn the_fixture_clap_and_sps_crop_disagree() {
    let mut bytes = test_fixtures::read(FIXTURE);
    let at = bytes.windows(4).position(|w| w == b"clap").unwrap();
    bytes[at..at + 4].copy_from_slice(b"cla0");
    let codec_only = display_of(&bytes);
    assert_eq!((codec_only.coded_width, codec_only.coded_height), CODED);
    assert_eq!(codec_only.output_rect, sps_crop());
}

#[test]
fn clap_replaces_the_sps_crop_in_output_rect() {
    let display = display_of(&test_fixtures::read(FIXTURE));
    assert_eq!((display.coded_width, display.coded_height), CODED);
    assert_eq!(display.output_rect, rect(8, 4, 1904, 1072));
}

#[test]
fn signed_offsets_move_the_aperture() {
    let bytes = with_clap((1904, 1), (1072, 1), (-4, 1), (2, 1));
    assert_eq!(display_of(&bytes).output_rect, rect(4, 6, 1904, 1072));

    let bytes = with_clap((1904, 1), (1072, 1), (-8, 1), (-4, 1));
    assert_eq!(display_of(&bytes).output_rect, rect(0, 0, 1904, 1072));
}

#[test]
fn fractions_that_reduce_to_whole_samples_apply() {
    let bytes = with_clap((3808, 2), (2144, 2), (-10, 5), (6, 3));
    assert_eq!(display_of(&bytes).output_rect, rect(6, 6, 1904, 1072));
}

#[test]
fn an_aperture_that_is_not_whole_samples_keeps_the_codec_crop() {
    for (case, bytes) in [
        (
            "odd width gives a half-sample left edge",
            with_clap((1903, 1), (1072, 1), (0, 1), (0, 1)),
        ),
        (
            "fractional width",
            with_clap((3807, 2), (1072, 1), (0, 1), (0, 1)),
        ),
        (
            "half-sample offset",
            with_clap((1904, 1), (1072, 1), (1, 2), (0, 1)),
        ),
    ] {
        assert_eq!(display_of(&bytes).output_rect, sps_crop(), "{case}");
    }
}

#[test]
fn an_invalid_or_out_of_frame_clap_keeps_the_codec_crop() {
    for (case, bytes) in [
        (
            "aperture left of the frame",
            with_clap((1904, 1), (1072, 1), (-9, 1), (0, 1)),
        ),
        (
            "aperture right of the frame",
            with_clap((1904, 1), (1072, 1), (9, 1), (0, 1)),
        ),
        (
            "aperture wider than the frame",
            with_clap((2000, 1), (1072, 1), (0, 1), (0, 1)),
        ),
        ("zero width", with_clap((0, 1), (1072, 1), (0, 1), (0, 1))),
        (
            "negative height",
            with_clap((1904, 1), (-1072, 1), (0, 1), (0, 1)),
        ),
        (
            "zero denominator",
            with_clap((1904, 0), (1072, 1), (0, 1), (0, 1)),
        ),
        (
            "negative denominator",
            with_clap((1904, 1), (1072, 1), (0, -1), (0, 1)),
        ),
    ] {
        assert_eq!(display_of(&bytes).output_rect, sps_crop(), "{case}");
    }
}
