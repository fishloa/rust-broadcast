//! HDR10+ (ST 2094-40) over the real Tears of Steel stream
//! `private/fixtures/video/hdr10plus/` (see its PROVENANCE.md): typed parse of
//! every SEI against the x265/hdr10plus_tool JSON, byte-exact round trip of all
//! 259 SEI NAL units, and the multi-window / Bezier / peak-luminance branches
//! the stream does not exercise, built from values chosen from the semantics
//! (no real capture carries them).
//!
//! Spec inputs: `private/specs-verified/smpte/hdr10plus/`.

use broadcast_bitstream::annexb::iter_annexb_nals;
use broadcast_bitstream::{
    AdditionalWindow, BezierToneMapping, ColorVolumeTransform, DistributionPoint, Error,
    Hdr10PlusMetadata, OverlapProcessOption, PeakLuminanceTable, WindowRegion,
};
use broadcast_core::error::InvalidValue;
use broadcast_core::{Parse, Serialize};
use test_fixtures::bits::BitWriter;

const HEVC: &str = "video/hdr10plus/tos-256x144-hdr10plus.hevc";
const FRAMES: usize = 259;

fn hdr10plus_sei_nals(es: &[u8]) -> Vec<Vec<u8>> {
    iter_annexb_nals(es)
        .filter(|n| Hdr10PlusMetadata::hevc_sei_nal_carries_hdr10plus(n))
        .map(<[u8]>::to_vec)
        .collect()
}

fn fixture_es() -> Option<Vec<u8>> {
    test_fixtures::require_bytes(HEVC, "hdr10plus")
}

#[test]
fn every_sei_in_the_stream_round_trips_byte_exact() {
    let Some(es) = fixture_es() else { return };
    let nals = hdr10plus_sei_nals(&es);
    assert_eq!(nals.len(), FRAMES);
    for (i, nal) in nals.iter().enumerate() {
        let payloads = Hdr10PlusMetadata::hevc_sei_nal_payloads(
            nal,
            &mut broadcast_bitstream::Findings::new(),
        );
        assert_eq!(payloads.len(), 1, "SEI {i}");
        let meta =
            Hdr10PlusMetadata::parse(&payloads[0]).unwrap_or_else(|e| panic!("SEI {i}: {e}"));
        assert_eq!(meta.try_to_bytes().unwrap(), payloads[0], "payload {i}");
        assert_eq!(meta.to_hevc_prefix_sei_nal().unwrap(), *nal, "NAL {i}");
        let again = Hdr10PlusMetadata::parse(&meta.try_to_bytes().unwrap()).unwrap();
        assert_eq!(again, meta, "serialize then parse {i}");
    }
}

#[test]
fn first_sei_decodes_to_the_values_ffprobe_reports() {
    let Some(es) = fixture_es() else { return };
    let nal = &hdr10plus_sei_nals(&es)[0];
    let meta = Hdr10PlusMetadata::from_hevc_sei_nal(nal, &mut broadcast_bitstream::Findings::new())
        .unwrap();
    assert_eq!(meta.application_version(), 1);
    assert_eq!(meta.num_windows(), 1);
    assert_eq!(meta.targeted_system_display_maximum_luminance(), 0);
    assert!(
        meta.targeted_system_display_actual_peak_luminance()
            .is_none()
    );
    assert!(meta.mastering_display_actual_peak_luminance().is_none());
    let w = meta.primary_window();
    assert_eq!(w.max_scl(), [17830, 16895, 14252]);
    assert_eq!(w.average_max_rgb(), 1037);
    let dist: Vec<(u8, u32)> = w
        .distribution_max_rgb()
        .iter()
        .map(|p| (p.percentage(), p.percentile()))
        .collect();
    assert_eq!(
        dist,
        [
            (1, 3),
            (5, 14024),
            (10, 43),
            (25, 56),
            (50, 219),
            (75, 1036),
            (90, 2714),
            (95, 4668),
            (99, 14445)
        ]
    );
    assert_eq!(w.fraction_bright_pixels(), 0);
    assert!(w.tone_mapping().is_none());
    assert!(w.color_saturation_weight().is_none());
    assert_eq!(meta.alignment_padding(), 0);
}

#[test]
fn the_stream_violates_table_1_and_is_still_accepted() {
    let Some(es) = fixture_es() else { return };
    let meta = Hdr10PlusMetadata::from_hevc_sei_nal(
        &hdr10plus_sei_nals(&es)[0],
        &mut broadcast_bitstream::Findings::new(),
    )
    .unwrap();
    let d = meta.primary_window().distribution_max_rgb();
    assert_eq!((d[1].percentage(), d[2].percentage()), (5, 10));
    assert_ne!(d[1].percentile(), 0);
    assert_ne!(d[2].percentile(), 255);
}

fn transform(seed: u32) -> ColorVolumeTransform {
    ColorVolumeTransform::new(
        [seed, seed + 1, seed + 2],
        seed * 3,
        (0..6u32)
            .map(|k| DistributionPoint::new((k * 20) as u8, seed * 100 + k))
            .collect(),
        77,
    )
}

fn region(base: u16) -> WindowRegion {
    WindowRegion::builder()
        .upper_left_corner_x(base)
        .upper_left_corner_y(base + 1)
        .lower_right_corner_x(base + 200)
        .lower_right_corner_y(base + 120)
        .center_of_ellipse_x(base + 100)
        .center_of_ellipse_y(base + 60)
        .rotation_angle(180)
        .semimajor_axis_internal_ellipse(40)
        .semimajor_axis_external_ellipse(65535)
        .semiminor_axis_external_ellipse(30)
        .overlap_process_option(OverlapProcessOption::Layering)
        .build()
}

fn round_trip(meta: &Hdr10PlusMetadata) -> Hdr10PlusMetadata {
    let bytes = meta.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), meta.serialized_len());
    assert_eq!(&bytes[..6], &[0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04]);
    let parsed = Hdr10PlusMetadata::parse(&bytes).unwrap();
    assert_eq!(&parsed, meta);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    let nal = meta.to_hevc_prefix_sei_nal().unwrap();
    assert_eq!(
        Hdr10PlusMetadata::from_hevc_sei_nal(&nal, &mut broadcast_bitstream::Findings::new())
            .unwrap(),
        *meta
    );
    parsed
}

#[test]
fn constructed_three_windows_with_bezier_and_tables_round_trip() {
    let primary = transform(10)
        .with_tone_mapping(BezierToneMapping::new(
            4095,
            1,
            (0..9u16).map(|k| k * 100 + 3).collect(),
        ))
        .with_color_saturation_weight(63);
    let second = transform(20).with_tone_mapping(BezierToneMapping::new(0, 0, Vec::new()));
    let third = transform(30).with_color_saturation_weight(0);
    let meta = Hdr10PlusMetadata::new(1, (1 << 27) - 1, primary)
        .with_targeted_system_display_actual_peak_luminance(PeakLuminanceTable::new(
            2,
            3,
            vec![0, 15, 7, 8, 1, 14],
        ))
        .with_additional_windows(vec![
            AdditionalWindow::new(region(5), second),
            AdditionalWindow::new(
                region(9).with_overlap_process_option(OverlapProcessOption::WeightedAverage),
                third,
            ),
        ])
        .with_mastering_display_actual_peak_luminance(PeakLuminanceTable::new(
            25,
            25,
            (0..625u32).map(|k| (k % 16) as u8).collect(),
        ));
    let parsed = round_trip(&meta);
    assert_eq!(parsed.num_windows(), 3);
    assert_eq!(
        parsed.additional_windows()[0].region().rotation_angle(),
        180
    );
    assert_eq!(
        parsed.additional_windows()[1]
            .region()
            .overlap_process_option(),
        OverlapProcessOption::WeightedAverage
    );
    assert_eq!(
        parsed
            .primary_window()
            .tone_mapping()
            .unwrap()
            .anchors()
            .len(),
        9
    );
    assert_eq!(
        parsed
            .mastering_display_actual_peak_luminance()
            .unwrap()
            .values()
            .len(),
        625
    );
}

#[test]
fn version_zero_with_fifteen_anchors_and_fifteen_percentiles_round_trips() {
    let primary = transform(5)
        .with_distribution_max_rgb(
            (0..15u32)
                .map(|k| DistributionPoint::new((k * 7) as u8, (1 << 17) - 1 - k))
                .collect(),
        )
        .with_tone_mapping(BezierToneMapping::new(
            2048,
            2047,
            (0..15u16).map(|k| 1023 - k).collect(),
        ));
    let meta = Hdr10PlusMetadata::new(0, 1000, primary);
    round_trip(&meta);
}

#[test]
fn alignment_padding_bits_are_preserved() {
    let meta = Hdr10PlusMetadata::new(1, 0, transform(1));
    let mut bytes = meta.try_to_bytes().unwrap();
    let last = bytes.len() - 1;
    bytes[last] |= 0b11;
    let parsed = Hdr10PlusMetadata::parse(&bytes).unwrap();
    assert_eq!(parsed.alignment_padding(), 0b11);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn malformed_payloads_are_rejected() {
    let meta = Hdr10PlusMetadata::new(1, 0, transform(1));
    let good = meta.try_to_bytes().unwrap();
    for cut in 0..good.len() {
        assert!(
            matches!(
                Hdr10PlusMetadata::parse(&good[..cut]),
                Err(Error::BufferTooShort(..))
            ),
            "cut {cut}"
        );
    }
    let mut extra = good.clone();
    extra.push(0);
    assert!(matches!(
        Hdr10PlusMetadata::parse(&extra),
        Err(Error::InvalidValue(..))
    ));
    let mut zero_windows = good.clone();
    zero_windows[7] &= 0b0011_1111;
    assert!(matches!(
        Hdr10PlusMetadata::parse(&zero_windows),
        Err(Error::InvalidValue(..))
    ));
    let mut wrong_provider = good.clone();
    wrong_provider[2] = 0x3D;
    assert!(matches!(
        Hdr10PlusMetadata::parse(&wrong_provider),
        Err(Error::InvalidValue(..))
    ));
    let mut wrong_app = good.clone();
    wrong_app[5] = 5;
    assert!(matches!(
        Hdr10PlusMetadata::parse(&wrong_app),
        Err(Error::InvalidValue(..))
    ));
    let mut wrong_version = good;
    wrong_version[6] = 2;
    assert!(matches!(
        Hdr10PlusMetadata::parse(&wrong_version),
        Err(Error::InvalidValue(..))
    ));
}

#[test]
fn serializing_inconsistent_values_is_an_error_not_a_misframed_payload() {
    let base = Hdr10PlusMetadata::new(1, 0, transform(1));
    let mut scl = base.primary_window().max_scl();
    scl[0] = 1 << 17;
    let too_wide = base
        .clone()
        .with_primary_window(base.primary_window().clone().with_max_scl(scl));
    assert!(matches!(
        too_wide.try_to_bytes(),
        Err(Error::FieldOverflow(_))
    ));
    let bad_table = base
        .clone()
        .with_mastering_display_actual_peak_luminance(PeakLuminanceTable::new(2, 2, vec![0; 3]));
    assert!(matches!(
        bad_table.try_to_bytes(),
        Err(Error::InvalidValue(InvalidValue {
            field: "mastering_display_actual_peak_luminance",
            ..
        }))
    ));
    let mut windows = Vec::new();
    for k in 0..3 {
        windows.push(AdditionalWindow::new(region(k), transform(2)));
    }
    let too_many_windows = base.clone().with_additional_windows(windows);
    assert!(matches!(
        too_many_windows.try_to_bytes(),
        Err(Error::InvalidValue(InvalidValue {
            field: "num_windows",
            value: 4,
            ..
        }))
    ));
    let too_many_points = base.clone().with_primary_window(
        base.primary_window().clone().with_distribution_max_rgb(
            (0..16u32)
                .map(|k| DistributionPoint::new(k as u8, k))
                .collect(),
        ),
    );
    assert!(matches!(
        too_many_points.try_to_bytes(),
        Err(Error::FieldOverflow(_))
    ));
}

fn no_lut_metadata() -> Hdr10PlusMetadata {
    Hdr10PlusMetadata::new(1, 0, transform(1))
}

/// Write `bits` bits of `value` at body bit `from`, counting from the first
/// payload bit after the 7-byte T.35 header.
fn set_payload_bits(bytes: &mut [u8], from: usize, bits: usize, value: u64) {
    for i in 0..bits {
        let bit_at = from + i;
        let mask = 1u8 << (7 - (bit_at % 8));
        let byte = &mut bytes[7 + bit_at / 8];
        if (value >> (bits - 1 - i)) & 1 == 1 {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }
}

/// ST 2094-40 clause 7.2 bounds a peak-luminance table to 25 rows and 25
/// columns: one dimension over the bound is rejected, both when writing and
/// when reading.
#[test]
fn a_lut_over_25_in_one_dimension_is_rejected() {
    for table in [
        PeakLuminanceTable::new(26, 1, vec![0; 26]),
        PeakLuminanceTable::new(1, 26, vec![0; 26]),
    ] {
        let meta = no_lut_metadata().with_mastering_display_actual_peak_luminance(table);
        assert!(
            matches!(
                meta.try_to_bytes(),
                Err(Error::InvalidValue(InvalidValue {
                    field: "mastering_display_actual_peak_luminance",
                    ..
                }))
            ),
            "{meta:?}"
        );
    }

    let meta = no_lut_metadata()
        .with_targeted_system_display_actual_peak_luminance(PeakLuminanceTable::new(1, 1, vec![3]));
    let good = meta.try_to_bytes().unwrap();
    // Body bits: num_windows(2), targeted_max_luminance(27), flag(1), rows(5),
    // columns(5).
    let mut rows_26 = good.clone();
    set_payload_bits(&mut rows_26, 30, 5, 26);
    assert!(
        matches!(
            Hdr10PlusMetadata::parse(&rows_26),
            Err(Error::InvalidValue(InvalidValue {
                field: "targeted_system_display_actual_peak_luminance",
                ..
            }))
        ),
        "26 rows must be rejected"
    );
    let mut columns_26 = good;
    set_payload_bits(&mut columns_26, 35, 5, 26);
    assert!(
        matches!(
            Hdr10PlusMetadata::parse(&columns_26),
            Err(Error::InvalidValue(InvalidValue {
                field: "targeted_system_display_actual_peak_luminance",
                ..
            }))
        ),
        "26 columns must be rejected"
    );
}

/// The rejections name the field and the offending value, including the
/// count of trailing bytes after the defined syntax.
#[test]
fn parse_errors_name_the_field_and_value() {
    let mut wrong_app = no_lut_metadata().try_to_bytes().unwrap();
    wrong_app[5] = 3;
    assert!(
        matches!(
            Hdr10PlusMetadata::parse(&wrong_app),
            Err(Error::InvalidValue(InvalidValue {
                field: "application_identifier",
                value: 3,
                ..
            }))
        ),
        "{:?}",
        Hdr10PlusMetadata::parse(&wrong_app)
    );

    let mut trailing = no_lut_metadata().try_to_bytes().unwrap();
    trailing.extend_from_slice(&[0x00, 0x00]);
    assert!(
        matches!(
            Hdr10PlusMetadata::parse(&trailing),
            Err(Error::InvalidValue(InvalidValue {
                field: "HDR10+ payload",
                value: 2,
                ..
            }))
        ),
        "{:?}",
        Hdr10PlusMetadata::parse(&trailing)
    );
}

/// A payload whose syntax ends on a byte boundary has no padding bits, so a
/// non-zero `alignment_padding` is an error rather than dropped bytes.
#[test]
fn non_zero_padding_on_a_byte_aligned_payload_is_an_error() {
    let primary = transform(1).with_tone_mapping(BezierToneMapping::new(0, 0, Vec::new()));
    let second = transform(2).with_tone_mapping(BezierToneMapping::new(0, 0, Vec::new()));
    let meta = Hdr10PlusMetadata::new(1, 0, primary)
        .with_additional_windows(vec![AdditionalWindow::new(region(5), second)])
        .with_alignment_padding(1);
    assert!(
        matches!(
            meta.try_to_bytes(),
            Err(Error::InvalidValue(InvalidValue {
                field: "alignment padding",
                value: 1,
                ..
            }))
        ),
        "a byte-aligned payload must carry no padding bits"
    );
    let meta = meta.with_alignment_padding(0);
    round_trip(&meta);
}
fn window_color_volume(w: &mut BitWriter, seed: u64) {
    w.u(17, 100 + seed).u(17, 200 + seed).u(17, 300 + seed);
    w.u(17, 400 + seed);
    w.u(4, 2);
    w.u(7, 1).u(17, 1000 + seed);
    w.u(7, 99).u(17, 131071);
    w.u(10, 777 + seed);
}

fn payload_with(bits: BitWriter) -> Vec<u8> {
    let mut out = vec![0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04, 0x01];
    out.extend_from_slice(&bits.finish());
    out
}

#[test]
fn a_payload_assembled_independently_field_by_field_decodes_to_those_fields() {
    let mut b = BitWriter::new();
    b.u(2, 1);
    b.u(27, 1000);
    b.u(1, 1).u(5, 2).u(5, 3);
    for v in [0, 1, 2, 3, 14, 15] {
        b.u(4, v);
    }
    window_color_volume(&mut b, 0);
    b.u(1, 1).u(5, 1).u(5, 2).u(4, 9).u(4, 15);
    b.u(1, 1)
        .u(12, 4095)
        .u(12, 1234)
        .u(4, 2)
        .u(10, 1023)
        .u(10, 5);
    b.u(1, 1).u(6, 63);
    let bytes = payload_with(b);
    let meta = Hdr10PlusMetadata::parse(&bytes).unwrap();
    assert_eq!(meta.targeted_system_display_maximum_luminance(), 1000);
    let lut = meta
        .targeted_system_display_actual_peak_luminance()
        .unwrap();
    assert_eq!((lut.rows(), lut.columns()), (2, 3));
    assert_eq!(lut.values(), [0, 1, 2, 3, 14, 15]);
    let w = meta.primary_window();
    assert_eq!(w.max_scl(), [100, 200, 300]);
    assert_eq!(w.average_max_rgb(), 400);
    assert_eq!(
        (
            w.distribution_max_rgb()[0].percentage(),
            w.distribution_max_rgb()[0].percentile()
        ),
        (1, 1000)
    );
    assert_eq!(
        (
            w.distribution_max_rgb()[1].percentage(),
            w.distribution_max_rgb()[1].percentile()
        ),
        (99, 131071)
    );
    assert_eq!(w.fraction_bright_pixels(), 777);
    let mastering = meta.mastering_display_actual_peak_luminance().unwrap();
    assert_eq!((mastering.rows(), mastering.columns()), (1, 2));
    assert_eq!(mastering.values(), [9, 15]);
    let tone = w.tone_mapping().unwrap();
    assert_eq!((tone.knee_point_x(), tone.knee_point_y()), (4095, 1234));
    assert_eq!(tone.anchors(), [1023, 5]);
    assert_eq!(w.color_saturation_weight(), Some(63));
    assert_eq!(meta.try_to_bytes().unwrap(), bytes);
}

#[test]
fn a_three_window_payload_assembled_independently_decodes_to_those_fields() {
    let mut b = BitWriter::new();
    b.u(2, 3);
    for base in [10u64, 20] {
        for v in 0..6 {
            b.u(16, base + v);
        }
        b.u(8, 90 + base);
        b.u(16, 1000 + base).u(16, 2000 + base).u(16, 3000 + base);
        b.u(1, base % 2);
    }
    b.u(27, 5).u(1, 0);
    for seed in 0..3 {
        window_color_volume(&mut b, seed);
    }
    b.u(1, 0);
    b.u(1, 0).u(1, 0);
    b.u(1, 1).u(12, 0).u(12, 0).u(4, 0).u(1, 0);
    b.u(1, 0).u(1, 1).u(6, 8);
    let bytes = payload_with(b);
    let meta = Hdr10PlusMetadata::parse(&bytes).unwrap();
    assert_eq!(meta.num_windows(), 3);
    let first = meta.additional_windows()[0].region();
    assert_eq!(
        (
            first.upper_left_corner_x(),
            first.upper_left_corner_y(),
            first.lower_right_corner_x(),
            first.lower_right_corner_y(),
            first.center_of_ellipse_x(),
            first.center_of_ellipse_y(),
            first.rotation_angle(),
            first.semimajor_axis_internal_ellipse(),
            first.semimajor_axis_external_ellipse(),
            first.semiminor_axis_external_ellipse(),
        ),
        (10, 11, 12, 13, 14, 15, 100, 1010, 2010, 3010)
    );
    assert_eq!(
        first.overlap_process_option(),
        OverlapProcessOption::WeightedAverage
    );
    let second = meta.additional_windows()[1].region();
    assert_eq!(second.semiminor_axis_external_ellipse(), 3020);
    assert_eq!(
        second.overlap_process_option(),
        OverlapProcessOption::WeightedAverage
    );
    assert_eq!(
        meta.additional_windows()[1].color_volume().max_scl(),
        [102, 202, 302]
    );
    assert_eq!(
        meta.additional_windows()[0]
            .color_volume()
            .fraction_bright_pixels(),
        778
    );
    assert!(meta.primary_window().tone_mapping().is_none());
    assert_eq!(
        meta.additional_windows()[0]
            .color_volume()
            .tone_mapping()
            .unwrap()
            .anchors()
            .len(),
        0
    );
    assert_eq!(
        meta.additional_windows()[1]
            .color_volume()
            .color_saturation_weight(),
        Some(8)
    );
    assert_eq!(meta.try_to_bytes().unwrap(), bytes);
}

#[test]
fn sample_walk_rejects_a_maximal_nal_length_without_overflow() {
    let sample = [0xFF, 0xFF, 0xFF, 0xFF, 0x4E, 0x01];
    assert!(!Hdr10PlusMetadata::sample_carries_hdr10plus(&sample));
}
