//! Spec vectors for the H.262 display syntax, hand-built from the transcription
//! (`private/specs-verified/mpeg/transmux/codec/mpeg2-video-display.md`), never from production code.

use super::*;
use crate::test_bits::BitWriter;
use alloc::vec::Vec;

fn sequence_header(width: u64, height: u64, aspect: u64) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u(12, width)
        .u(12, height)
        .u(4, aspect)
        .u(4, 3) // frame_rate_code
        .u(18, 0x3FFFF) // bit_rate_value
        .u(1, 1) // marker_bit
        .u(10, 112) // vbv_buffer_size_value
        .u(1, 0) // constrained_parameters_flag
        .u(1, 0) // load_intra_quantiser_matrix
        .u(1, 0); // load_non_intra_quantiser_matrix
    let mut out = SEQUENCE_HEADER_CODE.to_vec();
    out.extend_from_slice(&w.finish());
    out
}

fn sequence_extension(hx: u64, vx: u64) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u(4, u64::from(SEQUENCE_EXTENSION_ID))
        .u(8, 0x48) // profile_and_level_indication: Main @ Main
        .u(1, 0) // progressive_sequence
        .u(2, 1) // chroma_format: 4:2:0
        .u(2, hx)
        .u(2, vx)
        .u(12, 0); // bit_rate_extension
    let mut out = vec![0x00, 0x00, 0x01, EXTENSION_START_CODE_BYTE];
    out.extend_from_slice(&w.finish());
    out
}

fn sequence_display_extension(
    video_format: u64,
    colour: Option<(u64, u64, u64)>,
    width: u64,
    height: u64,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u(4, u64::from(SEQUENCE_DISPLAY_EXTENSION_ID))
        .u(3, video_format)
        .u(1, u64::from(colour.is_some()));
    if let Some((p, t, m)) = colour {
        w.u(8, p).u(8, t).u(8, m);
    }
    w.u(14, width).u(1, 1).u(14, height);
    let mut out = vec![0x00, 0x00, 0x01, EXTENSION_START_CODE_BYTE];
    out.extend_from_slice(&w.finish());
    out
}

fn geometry(parts: &[Vec<u8>]) -> DisplayGeometry {
    let es: Vec<u8> = parts.concat();
    DisplayGeometry::from_mpeg2_es(&es).expect("geometry")
}

fn sar(g: &DisplayGeometry) -> Option<(u16, u16)> {
    g.sample_aspect_ratio()
}

#[test]
fn aspect_codes_derive_the_table_6_3_ratios() {
    assert_eq!(
        sar(&geometry(&[sequence_header(704, 480, 1)])),
        Some((1, 1))
    );
    assert_eq!(
        sar(&geometry(&[sequence_header(704, 480, 2)])),
        Some((10, 11))
    );
    assert_eq!(
        sar(&geometry(&[sequence_header(704, 576, 3)])),
        Some((16, 11))
    );
    assert_eq!(
        sar(&geometry(&[sequence_header(720, 576, 3)])),
        Some((64, 45))
    );
    assert_eq!(
        sar(&geometry(&[sequence_header(720, 576, 2)])),
        Some((16, 15))
    );
    // 2.21:1 on 720x576: (221/100) * 576 / 720 = 1768/1000 = 221/125.
    assert_eq!(
        sar(&geometry(&[sequence_header(720, 576, 4)])),
        Some((221, 125))
    );
}

#[test]
fn forbidden_and_reserved_aspects_are_unspecified() {
    for aspect in core::iter::once(0u64).chain(FIRST_RESERVED_ASPECT as u64..16) {
        let g = geometry(&[sequence_header(704, 480, aspect)]);
        assert_eq!(g.aspect_ratio, None, "aspect {aspect}");
        assert_eq!(g.sample_aspect_ratio(), None);
    }
}

#[test]
fn sequence_extension_extends_the_coded_size() {
    let g = geometry(&[sequence_header(111, 222, 1), sequence_extension(1, 2)]);
    assert_eq!(
        (g.coded_width, g.coded_height),
        (1 << 12 | 111, 2 << 12 | 222)
    );
    assert_eq!(g.output_rect.width, 1 << 12 | 111);
}

#[test]
fn square_samples_are_square() {
    let g = geometry(&[sequence_header(320, 240, 1)]);
    assert_eq!(g.aspect_ratio.map(|a| a.idc()), Some(1));
    assert_eq!(g.sample_aspect_ratio(), Some((1, 1)));
}

#[test]
fn display_extension_colour_is_signalled_and_inferred_is_bt709() {
    let plain = geometry(&[sequence_header(704, 480, 2)]);
    assert_eq!(plain.video_signal, None);
    assert!(plain.is_mpeg2());
    assert_eq!(
        plain.colour_description(),
        ColourDescription::from_wire(1, 1, 1)
    );
    let signalled = geometry(&[
        sequence_header(704, 480, 2),
        sequence_display_extension(1, Some((6, 6, 5)), 704, 480),
    ]);
    assert_eq!(
        signalled.colour_description(),
        ColourDescription::from_wire(6, 6, 5)
    );
    let format_only = geometry(&[
        sequence_header(704, 480, 2),
        sequence_display_extension(2, None, 704, 480),
    ]);
    assert_eq!(
        format_only.video_format(),
        VideoFormat::Ntsc,
        "video_format 2 is NTSC (Table 6-6)"
    );
    assert_eq!(
        format_only.colour_description(),
        ColourDescription::from_wire(1, 1, 1),
        "colour_description 0 infers BT.709"
    );
    assert!(!format_only.full_range(), "H.262 has no full-range flag");
}

#[test]
fn display_extension_size_feeds_the_spec_relation() {
    // H.262 §6.3.3: with sequence_display_extension() present the relation
    // uses display_horizontal_size / display_vertical_size.
    let g = geometry(&[
        sequence_header(720, 576, 3),
        sequence_display_extension(5, None, 704, 576),
    ]);
    assert_eq!(g.coded_width, 720);
    assert_eq!(g.output_rect.width, 720, "no crop syntax in H.262");
    // (16/9) * 576 / 704 = 16/11.
    assert_eq!(g.sample_aspect_ratio(), Some((16, 11)));
}

#[test]
fn no_sequence_header_is_an_error() {
    assert!(matches!(
        DisplayGeometry::from_mpeg2_es(&[0x00, 0x00, 0x01, 0x00, 0x08]),
        Err(crate::error::Error::InvalidInput(..))
    ));
}

#[test]
fn codec_config_without_sequence_header_yields_none() {
    use crate::mp4esds::{
        DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
    };
    let esds = EsdsBox::new(ESDescriptor::new(
        0,
        0,
        Some(DecoderConfigDescriptor::new(
            crate::ts_demux::OTI_MPEG2_VIDEO_MAIN,
            crate::ts_demux::STREAM_TYPE_VISUAL,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(vec![0x55])),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ));
    let config = crate::ir::CodecConfig::Mpeg2Video {
        esds,
        width: 704,
        height: 480,
    };
    assert_eq!(DisplayGeometry::from_codec_config(&config), None);
}

/// The FIRST sequence header a real capture carries still parses to the
/// capture's own geometry (the first header of a stream is the one its
/// opening pictures use). It is a weaker oracle than the demux-level tests:
/// a capture may open with a sequence header that differs from the one a
/// later picture uses.
#[test]
fn real_capture_sequence_header_matches_the_ffprobe_oracle() {
    for (fixture, expect) in [
        // ffprobe: 704x576 16:11 (SAR 16:11), display extension 704x576.
        (
            "suites/tsduck/input/test-205.ts",
            (704u32, 576u32, (16u16, 11u16)),
        ),
        ("ts/legacy/mpeg2-mp2.ts", (320, 240, (1, 1))),
    ] {
        let Some(ts) = test_fixtures::require_bytes(fixture, "h262") else {
            return;
        };
        let g = DisplayGeometry::from_mpeg2_es(&ts).expect("geometry");
        assert_eq!(
            (g.coded_width, g.coded_height, g.sample_aspect_ratio()),
            (expect.0, expect.1, Some(expect.2)),
            "{fixture}"
        );
    }
}

#[test]
fn codec_config_sequence_header_gives_geometry() {
    use crate::mp4esds::{
        DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
    };
    let es = sequence_header(704, 576, 3);
    let esds = EsdsBox::new(ESDescriptor::new(
        0,
        0,
        Some(DecoderConfigDescriptor::new(
            crate::ts_demux::OTI_MPEG2_VIDEO_MAIN,
            crate::ts_demux::STREAM_TYPE_VISUAL,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(es)),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ));
    let config = crate::ir::CodecConfig::Mpeg2Video {
        esds,
        width: 704,
        height: 576,
    };
    let g = DisplayGeometry::from_codec_config(&config).expect("esds sequence header");
    assert!(g.is_mpeg2());
    assert_eq!(g.sample_aspect_ratio(), Some((16, 11)));
}
