//! Display-geometry integration: the coded facts of an SPS read through the
//! bitstream crate and assembled by `transmux` into a presentation type.
//!
//! The SPS builders write the syntax in order (ITU-T H.264 §7.3.2.1.1 /
//! §E.1.1, ITU-T H.265 §7.3.2.2.1 / §E.2.1), so a mis-walk of any preceding
//! element shifts the bits the assertions read afterwards. The interlace
//! facts those same SPS builders feed live in `interlace`'s own test module.

use broadcast_bitstream::facts::{
    AspectRatioIdc, ColourDescription, ColourPrimaries, MatrixCoefficients,
    TransferCharacteristics, VideoFormat, VideoSignalType,
};
use broadcast_bitstream::sps::{decode_avc_sps, decode_hevc_interlace_sps};
use test_fixtures::bits::BitWriter;
use transmux::ir::DisplayGeometry;
use transmux::pipeline::{CodecConfig, FrameRateBasis, nominal_frame_rate};

type AspectBits = Option<(u8, Option<(u16, u16)>)>;

fn put_aspect(s: &mut BitWriter, aspect: AspectBits) {
    s.bit(aspect.is_some()); // aspect_ratio_info_present_flag
    if let Some((idc, ext)) = aspect {
        s.u(8, u64::from(idc)); // aspect_ratio_idc
        if let Some((w, h)) = ext {
            s.u(16, u64::from(w)); // sar_width
            s.u(16, u64::from(h)); // sar_height
        }
    }
}

fn hevc_sps_full(
    field_seq: bool,
    ffi: bool,
    vui: bool,
    ptl_bits: u64,
    aspect: AspectBits,
) -> Vec<u8> {
    let mut nal = vec![0x42u8, 0x01];
    let mut s = BitWriter::new();
    s.u(4, 0); // sps_video_parameter_set_id
    s.u(3, 0); // sps_max_sub_layers_minus1
    s.bit(true); // sps_temporal_id_nesting_flag
    s.u(2, 0); // general_profile_space
    s.bit(false); // general_tier_flag
    s.u(5, 1); // general_profile_idc
    s.u(32, 0); // general_profile_compatibility_flags
    s.u(48, ptl_bits); // general_constraint_indicator_flags
    s.u(8, 93); // general_level_idc
    s.ue(0); // sps_seq_parameter_set_id
    s.ue(1); // chroma_format_idc
    s.ue(16); // pic_width_in_luma_samples
    s.ue(16); // pic_height_in_luma_samples
    s.bit(false); // conformance_window_flag
    s.ue(0); // bit_depth_luma_minus8
    s.ue(0); // bit_depth_chroma_minus8
    s.ue(3); // log2_max_pic_order_cnt_lsb_minus4
    s.bit(false); // sps_sub_layer_ordering_info_present_flag
    s.ue(0);
    s.ue(0);
    s.ue(0); // sps_max_dec_pic_buffering/num_reorder/latency
    s.ue(0);
    s.ue(3);
    s.ue(0);
    s.ue(3); // coding/transform block sizes
    s.ue(1);
    s.ue(1); // max_transform_hierarchy_depth_inter/intra
    s.bit(false); // scaling_list_enabled_flag
    s.bit(false); // amp_enabled_flag
    s.bit(false); // sample_adaptive_offset_enabled_flag
    s.bit(false); // pcm_enabled_flag
    s.ue(3); // num_short_term_ref_pic_sets
    // set 0: explicit, DeltaPocS0 = [-1], DeltaPocS1 = [+2]
    s.ue(1); // num_negative_pics
    s.ue(1); // num_positive_pics
    s.ue(0); // delta_poc_s0_minus1 -> -1
    s.bit(true); // used_by_curr_pic_s0_flag
    s.ue(1); // delta_poc_s1_minus1 -> +2
    s.bit(true); // used_by_curr_pic_s1_flag
    // set 1: inter from set 0 (NumDeltaPocs = 2 -> 3 flag groups),
    // deltaRps = -3. Candidates: s1[0] -> -1 (j = 1, dropped below),
    // deltaRps entry (j = 2) -> -3, s0[0] -> -4 (j = 0): derives
    // S0 = [-3, -4], S1 = [] (2 entries).
    s.bit(true); // inter_ref_pic_set_prediction_flag
    s.bit(true); // delta_rps_sign
    s.ue(2); // abs_delta_rps_minus1
    s.bit(true); // used_by_curr_pic_flag[0]
    s.bit(false); // used_by_curr_pic_flag[1]
    s.bit(false); // use_delta_flag[1] = 0: -1 is not kept
    s.bit(true); // used_by_curr_pic_flag[2]
    // set 2: inter from set 1 (NumDeltaPocs = 2 -> 3 flags)
    s.bit(true); // inter_ref_pic_set_prediction_flag
    s.bit(false); // delta_rps_sign
    s.ue(0); // abs_delta_rps_minus1
    for _ in 0..3 {
        s.bit(true); // used_by_curr_pic_flag[j]
    }
    s.bit(false); // long_term_ref_pics_present_flag
    s.bit(false); // sps_temporal_mvp_enabled_flag
    s.bit(false); // strong_intra_smoothing_enabled_flag
    s.bit(vui); // vui_parameters_present_flag
    if vui {
        put_aspect(&mut s, aspect);
        s.bit(false); // overscan_info_present_flag
        s.bit(false); // video_signal_type_present_flag
        s.bit(false); // chroma_loc_info_present_flag
        s.bit(false); // neutral_chroma_indication_flag
        s.bit(field_seq); // field_seq_flag
        s.bit(ffi); // frame_field_info_present_flag
        s.bit(false); // default_display_window_flag
        s.bit(false); // vui_timing_info_present_flag
        s.bit(false); // bitstream_restriction_flag
    }
    s.bit(true); // rbsp_stop_one_bit
    nal.extend(s.finish());
    nal
}

const SPEC_TABLE_E1: [(u8, u16, u16); 16] = [
    (1, 1, 1),
    (2, 12, 11),
    (3, 10, 11),
    (4, 16, 11),
    (5, 40, 33),
    (6, 24, 11),
    (7, 20, 11),
    (8, 32, 11),
    (9, 80, 33),
    (10, 18, 11),
    (11, 15, 11),
    (12, 64, 33),
    (13, 160, 99),
    (14, 4, 3),
    (15, 3, 2),
    (16, 2, 1),
];

/// Baseline-profile 720x576 progressive SPS whose VUI carries `aspect`
/// then overscan/video-signal/chroma-loc absent and `timing_info` 1/50 (so
/// a mis-sized aspect read shifts the walk and corrupts the frame rate).
fn avc_display_sps(vui: bool, aspect: AspectBits) -> Vec<u8> {
    let mut nal = vec![0x67u8, 66, 0x00, 30];
    let mut s = BitWriter::new();
    s.ue(0); // seq_parameter_set_id
    s.ue(0); // log2_max_frame_num_minus4
    s.ue(2); // pic_order_cnt_type
    s.ue(1); // max_num_ref_frames
    s.bit(false); // gaps_in_frame_num_value_allowed_flag
    s.ue(44); // pic_width_in_mbs_minus1 -> 720
    s.ue(35); // pic_height_in_map_units_minus1 -> 576
    s.bit(true); // frame_mbs_only_flag
    s.bit(true); // direct_8x8_inference_flag
    s.bit(false); // frame_cropping_flag
    s.bit(vui); // vui_parameters_present_flag
    if vui {
        put_aspect(&mut s, aspect);
        s.bit(false); // overscan_info_present_flag
        s.bit(false); // video_signal_type_present_flag
        s.bit(false); // chroma_loc_info_present_flag
        s.bit(true); // timing_info_present_flag
        s.u(32, 1); // num_units_in_tick
        s.u(32, 50); // time_scale
        s.bit(true); // fixed_frame_rate_flag
        s.bit(false); // nal_hrd_parameters_present_flag
        s.bit(false); // vcl_hrd_parameters_present_flag
        s.bit(false); // pic_struct_present_flag
        s.bit(false); // bitstream_restriction_flag
    }
    s.bit(true); // rbsp_stop_one_bit
    nal.extend(s.finish());
    nal
}

fn avc_geometry(aspect: AspectBits) -> DisplayGeometry {
    let sps = avc_display_sps(true, aspect);
    // The VUI walk stays aligned past the aspect field: timing is intact.
    assert_eq!(decode_avc_sps(&sps).unwrap().fps(), Some(25.0));
    DisplayGeometry::from_avc_sps(&sps).unwrap()
}

fn hevc_geometry(aspect: AspectBits) -> DisplayGeometry {
    let sps = hevc_sps_full(true, true, true, 0, aspect);
    // The VUI walk stays aligned past the aspect field: field_seq_flag and
    // frame_field_info_present_flag (read after it) are intact.
    let i = decode_hevc_interlace_sps(&sps).unwrap();
    assert!(i.field_seq() && i.frame_field_info_present());
    DisplayGeometry::from_hevc_sps(&sps).unwrap()
}

#[test]
fn avc_every_table_e1_idc_decodes_to_its_ratio() {
    for (idc, w, h) in SPEC_TABLE_E1 {
        let g = avc_geometry(Some((idc, None)));
        assert_eq!(g.sample_aspect_ratio(), Some((w, h)), "H.264 idc {idc}");
        assert_eq!(g.aspect_ratio.unwrap().idc(), idc);
        assert_eq!((g.coded_width, g.coded_height), (720, 576));
    }
}

// -----------------------------------------------------------------
// Colour signalling: overscan + video_signal_type() (H.264 §E.1.1 /
// H.265 §E.2.1; Tables E-2..E-5 / E.2..E.5, transcribed in
// `private/specs-verified/mpeg/transmux/codec/vui-colour-tables.md`).
// -----------------------------------------------------------------

/// `(video_format, video_full_range_flag, colour_primaries/transfer/matrix)`.
type VideoSignalBits = (u8, bool, Option<(u8, u8, u8)>);

/// What a test wants in the VUI's overscan + video signal type.
#[derive(Clone, Copy, Default)]
struct ColourVui {
    overscan: Option<bool>,
    vst: Option<VideoSignalBits>,
}

/// Append `overscan_info_present_flag(..)` then `video_signal_type(..)`.
fn put_colour_vui(s: &mut BitWriter, v: ColourVui) {
    match v.overscan {
        Some(appropriate) => {
            s.bit(true); // overscan_info_present_flag
            s.bit(appropriate); // overscan_appropriate_flag
        }
        None => {
            s.bit(false);
        }
    }
    match v.vst {
        Some((format, full_range, colour)) => {
            s.bit(true); // video_signal_type_present_flag
            s.u(3, u64::from(format)); // video_format
            s.bit(full_range); // video_full_range_flag
            match colour {
                Some((primaries, transfer, matrix)) => {
                    s.bit(true); // colour_description_present_flag
                    s.u(8, u64::from(primaries));
                    s.u(8, u64::from(transfer));
                    s.u(8, u64::from(matrix));
                }
                None => {
                    s.bit(false);
                }
            }
        }
        None => {
            s.bit(false);
        }
    }
}

/// A colour description from three wire code points.
fn colour(primaries: u8, transfer: u8, matrix: u8) -> ColourDescription {
    ColourDescription::from_wire(primaries, transfer, matrix)
}

/// H.264 Table E-3 (H.265 E.3): defined rows -> primaries text.
fn avc_colour_sps(colour: ColourVui) -> Vec<u8> {
    let mut nal = vec![0x67u8, 66, 0x00, 30];
    let mut s = BitWriter::new();
    s.ue(0); // seq_parameter_set_id
    s.ue(0); // log2_max_frame_num_minus4
    s.ue(2); // pic_order_cnt_type
    s.ue(1); // max_num_ref_frames
    s.bit(false); // gaps_in_frame_num_value_allowed_flag
    s.ue(44); // pic_width_in_mbs_minus1 -> 720
    s.ue(35); // pic_height_in_map_units_minus1 -> 576
    s.bit(true); // frame_mbs_only_flag
    s.bit(true); // direct_8x8_inference_flag
    s.bit(false); // frame_cropping_flag
    s.bit(true); // vui_parameters_present_flag
    put_aspect(&mut s, Some((1, None)));
    put_colour_vui(&mut s, colour);
    s.bit(false); // chroma_loc_info_present_flag
    s.bit(true); // timing_info_present_flag
    s.u(32, 1); // num_units_in_tick
    s.u(32, 50); // time_scale
    s.bit(true); // fixed_frame_rate_flag
    s.bit(false); // nal_hrd_parameters_present_flag
    s.bit(false); // vcl_hrd_parameters_present_flag
    s.bit(false); // pic_struct_present_flag
    s.bit(false); // bitstream_restriction_flag
    s.bit(true); // rbsp_stop_one_bit
    nal.extend(s.finish());
    nal
}

/// The same VUI tail on an H.265 SPS: after the colour block come
/// `field_seq_flag`/`frame_field_info_present_flag`, read and asserted.
fn hevc_colour_sps(colour: ColourVui) -> Vec<u8> {
    let mut nal = vec![0x42u8, 0x01];
    let mut s = BitWriter::new();
    s.u(4, 0); // sps_video_parameter_set_id
    s.u(3, 0); // sps_max_sub_layers_minus1
    s.bit(true); // sps_temporal_id_nesting_flag
    s.u(2, 0); // general_profile_space
    s.bit(false); // general_tier_flag
    s.u(5, 1); // general_profile_idc
    s.u(32, 0); // general_profile_compatibility_flags
    s.u(48, 0); // general_constraint_indicator_flags
    s.u(8, 93); // general_level_idc
    s.ue(0); // sps_seq_parameter_set_id
    s.ue(1); // chroma_format_idc
    s.ue(16); // pic_width_in_luma_samples
    s.ue(16); // pic_height_in_luma_samples
    s.bit(false); // conformance_window_flag
    s.ue(0); // bit_depth_luma_minus8
    s.ue(0); // bit_depth_chroma_minus8
    s.ue(3); // log2_max_pic_order_cnt_lsb_minus4
    s.bit(false); // sps_sub_layer_ordering_info_present_flag
    s.ue(0); // sps_max_dec_pic_buffering_minus1
    s.ue(0); // sps_max_num_reorder_pics
    s.ue(0); // sps_max_latency_increase_plus1
    s.ue(0);
    s.ue(3);
    s.ue(0);
    s.ue(3); // coding/transform block sizes
    s.ue(1);
    s.ue(1); // max_transform_hierarchy_depth_inter/intra
    s.bit(false); // scaling_list_enabled_flag
    s.bit(false); // amp_enabled_flag
    s.bit(false); // sample_adaptive_offset_enabled_flag
    s.bit(false); // pcm_enabled_flag
    s.ue(0); // num_short_term_ref_pic_sets
    s.bit(false); // long_term_ref_pics_present_flag
    s.bit(false); // sps_temporal_mvp_enabled_flag
    s.bit(false); // strong_intra_smoothing_enabled_flag
    s.bit(true); // vui_parameters_present_flag
    put_aspect(&mut s, Some((1, None)));
    put_colour_vui(&mut s, colour);
    s.bit(false); // chroma_loc_info_present_flag
    s.bit(false); // neutral_chroma_indication_flag
    s.bit(true); // field_seq_flag
    s.bit(true); // frame_field_info_present_flag
    s.bit(false); // default_display_window_flag
    s.bit(false); // vui_timing_info_present_flag
    s.bit(false); // bitstream_restriction_flag
    s.bit(true); // rbsp_stop_one_bit
    nal.extend(s.finish());
    nal
}

/// Decode a colour VUI on both codecs and hand back the geometry.
fn both_colour_geometries(v: ColourVui) -> [DisplayGeometry; 2] {
    let avc = avc_colour_sps(v);
    assert_eq!(decode_avc_sps(&avc).unwrap().fps(), Some(25.0), "AVC walk");
    let hevc = hevc_colour_sps(v);
    let i = decode_hevc_interlace_sps(&hevc).expect("HEVC walk");
    assert!(
        i.field_seq() && i.frame_field_info_present(),
        "HEVC walk past the colour block"
    );
    [
        DisplayGeometry::from_avc_sps(&avc).unwrap(),
        DisplayGeometry::from_hevc_sps(&hevc).unwrap(),
    ]
}

#[test]
fn absent_video_signal_type_is_none_with_the_spec_inferred_defaults() {
    let g = both_colour_geometries(ColourVui::default());
    for g in g {
        assert_eq!(g.video_signal, None);
        assert_eq!(g.overscan_appropriate, None);
        assert!(!g.full_range());
        assert_eq!(g.video_format(), VideoFormat::Unspecified);
        let d = g.colour_description();
        assert_eq!(d.primaries(), ColourPrimaries::Unspecified);
        assert_eq!(d.transfer(), TransferCharacteristics::Unspecified);
        assert_eq!(d.matrix(), MatrixCoefficients::Unspecified);
    }
}

#[test]
fn video_signal_without_colour_description_keeps_colour_none() {
    let g = both_colour_geometries(ColourVui {
        overscan: None,
        vst: Some((0, true, None)),
    });
    for g in g {
        assert_eq!(
            g.video_signal,
            Some(VideoSignalType::new(VideoFormat::Component, true, None))
        );
        assert!(g.full_range());
        // Signalled format survives; colour falls back to the inference.
        assert_eq!(g.video_format(), VideoFormat::Component);
        assert_eq!(
            g.colour_description().matrix(),
            MatrixCoefficients::Unspecified
        );
    }
}

#[test]
fn full_colour_description_is_read_in_spec_order() {
    // 5/5/5 = melodie; 1/1/1 = BT.709; 9/16/9 = BT.2020 PQ. Distinct
    // codes catch a primaries/transfer/matrix read swap.
    for (p, t, m) in [(5u8, 5u8, 5u8), (1, 1, 1), (9, 16, 9)] {
        let g = both_colour_geometries(ColourVui {
            overscan: None,
            vst: Some((5, false, Some((p, t, m)))),
        });
        for g in g {
            assert_eq!(g.colour_description(), colour(p, t, m));
            assert_eq!(g.video_signal.unwrap().colour(), Some(colour(p, t, m)));
            assert!(!g.full_range());
        }
    }
}

#[test]
fn reserved_and_boundary_code_points_survive_the_walk() {
    for (p, t, m) in [(0u8, 0u8, 0u8), (3, 3, 3), (255, 255, 255), (17, 19, 18)] {
        let g = both_colour_geometries(ColourVui {
            overscan: None,
            vst: Some((5, false, Some((p, t, m)))),
        });
        for g in g {
            let d = g.colour_description();
            assert_eq!(
                (d.primaries().code(), d.transfer().code(), d.matrix().code()),
                (p, t, m)
            );
        }
    }
}

#[test]
fn overscan_flag_precedes_the_video_signal_type() {
    // The fixture that catches a swapped overscan/video_signal order: with
    // overscan present, a walk reading video_signal_type first reads the
    // overscan bits as the signal type and corrupts everything after it.
    for appropriate in [true, false] {
        let g = both_colour_geometries(ColourVui {
            overscan: Some(appropriate),
            vst: Some((5, true, Some((1, 1, 1)))),
        });
        for g in g {
            assert_eq!(g.overscan_appropriate, Some(appropriate));
            assert!(g.full_range());
            assert_eq!(g.colour_description(), colour(1, 1, 1));
        }
    }
    // Overscan present, video signal type absent.
    let g = both_colour_geometries(ColourVui {
        overscan: Some(true),
        vst: None,
    });
    for g in g {
        assert_eq!(g.overscan_appropriate, Some(true));
        assert_eq!(g.video_signal, None);
    }
}

/// H.265 has no real fixture with a non-square SAR: spec vectors only.
#[test]
fn hevc_every_table_e1_idc_decodes_to_its_ratio() {
    for (idc, w, h) in SPEC_TABLE_E1 {
        let g = hevc_geometry(Some((idc, None)));
        assert_eq!(g.sample_aspect_ratio(), Some((w, h)), "H.265 idc {idc}");
        assert_eq!(g.aspect_ratio.unwrap().idc(), idc);
        assert_eq!((g.coded_width, g.coded_height), (16, 16));
    }
}

#[test]
fn extended_sar_reads_width_then_height_for_both_codecs() {
    // 64:45 is the melodie fixture's value; width != height catches a swap.
    for g in [
        avc_geometry(Some((255, Some((64, 45))))),
        hevc_geometry(Some((255, Some((64, 45))))),
    ] {
        assert_eq!(
            g.aspect_ratio,
            Some(AspectRatioIdc::ExtendedSar {
                sar_width: 64,
                sar_height: 45
            })
        );
        assert_eq!(g.sample_aspect_ratio(), Some((64, 45)));
    }
    // Full 16-bit range, not truncated to 8 bits.
    let g = avc_geometry(Some((255, Some((0x1234, 0x0101)))));
    assert_eq!(g.sample_aspect_ratio(), Some((0x1234, 0x0101)));
}

#[test]
fn unspecified_ratios_are_none_never_a_zero_sentinel() {
    let cases: [AspectBits; 6] = [
        Some((0, None)),            // idc 0: Unspecified
        Some((17, None)),           // first reserved
        Some((254, None)),          // last reserved
        Some((255, Some((0, 45)))), // sar_width 0
        Some((255, Some((64, 0)))), // sar_height 0
        Some((255, Some((0, 0)))),  // both 0
    ];
    for c in cases {
        for g in [avc_geometry(c), hevc_geometry(c)] {
            // The wire value is kept, the ratio is unspecified.
            assert!(g.aspect_ratio.is_some(), "{c:?}");
            assert_eq!(g.sample_aspect_ratio(), None, "{c:?}");
            assert_eq!(g.typed_sample_aspect_ratio(), None, "{c:?}");
        }
    }
    assert_eq!(
        avc_geometry(Some((17, None))).aspect_ratio,
        Some(AspectRatioIdc::Reserved(17))
    );
    assert_eq!(
        avc_geometry(Some((254, None))).aspect_ratio,
        Some(AspectRatioIdc::Reserved(254))
    );
}

#[test]
fn absent_aspect_info_is_none_for_both_codecs() {
    // No VUI at all.
    let g = DisplayGeometry::from_avc_sps(&avc_display_sps(false, None)).unwrap();
    assert_eq!((g.aspect_ratio, g.sample_aspect_ratio()), (None, None));
    let hevc = hevc_sps_full(false, false, false, 0, None);
    let g = DisplayGeometry::from_hevc_sps(&hevc).unwrap();
    assert_eq!((g.aspect_ratio, g.sample_aspect_ratio()), (None, None));
    // VUI present, aspect_ratio_info_present_flag 0.
    assert_eq!(avc_geometry(None).aspect_ratio, None);
    assert_eq!(hevc_geometry(None).aspect_ratio, None);
}

#[test]
fn a_vui_truncated_inside_the_aspect_field_never_yields_a_half_read_value() {
    let whole = avc_display_sps(true, Some((255, Some((64, 45)))));
    let hevc_whole = hevc_sps_full(true, true, true, 0, Some((255, Some((64, 45)))));
    let full = Some(AspectRatioIdc::ExtendedSar {
        sar_width: 64,
        sar_height: 45,
    });
    let mut absent_seen = false;
    for cut in 0..whole.len() {
        // Never a panic, and the field is either intact or absent.
        if let Ok(g) = DisplayGeometry::from_avc_sps(&whole[..cut]) {
            assert!(
                g.aspect_ratio.is_none() || g.aspect_ratio == full,
                "avc cut {cut}"
            );
            absent_seen |= g.aspect_ratio.is_none();
        }
    }
    for cut in 0..hevc_whole.len() {
        if let Ok(g) = DisplayGeometry::from_hevc_sps(&hevc_whole[..cut]) {
            assert!(
                g.aspect_ratio.is_none() || g.aspect_ratio == full,
                "hevc cut {cut}"
            );
        }
    }
    // Not vacuous: some cut parsed the SPS but lost the field, and the
    // whole SPS carries it.
    assert!(absent_seen);
    let g = DisplayGeometry::from_avc_sps(&whole).unwrap();
    assert_eq!(g.aspect_ratio, full);
}

fn hevc_sps_vui_hrd(
    field_seq: bool,
    ffi: bool,
    num_units: u32,
    time_scale: u32,
    hrd: Option<(bool, bool, u32)>, // (general, within, ed_minus1)
) -> Vec<u8> {
    let mut nal = vec![0x42u8, 0x01];
    let mut s = BitWriter::new();
    s.u(4, 0); // vps id
    s.u(3, 0); // max sub layers minus1 = 0
    s.bit(true); // temporal id nesting
    s.u(2, 0); // profile space
    s.bit(false); // tier
    s.u(5, 1); // profile idc
    s.u(32, 0); // compat
    s.u(48, 0); // constraint
    s.u(8, 93); // level
    s.ue(0); // sps id
    s.ue(1); // chroma
    s.ue(16); // width
    s.ue(16); // height
    s.bit(false); // conf window
    s.ue(0); // bd luma
    s.ue(0); // bd chroma
    s.ue(3); // log2 poc
    s.bit(false); // sub layer ordering
    s.ue(0);
    s.ue(0);
    s.ue(0);
    s.ue(0);
    s.ue(3);
    s.ue(0);
    s.ue(3);
    s.ue(1);
    s.ue(1);
    s.bit(false); // scaling list
    s.bit(false); // amp
    s.bit(false); // sao
    s.bit(false); // pcm
    s.ue(0); // num short term
    s.bit(false); // ltrp
    s.bit(false); // temporal mvp
    s.bit(false); // strong intra
    s.bit(true); // vui present
    s.bit(false); // aspect ratio info present
    s.bit(false); // overscan
    s.bit(false); // video signal
    s.bit(false); // chroma loc
    s.bit(false); // neutral chroma
    s.bit(field_seq); // field_seq_flag
    s.bit(ffi); // frame_field_info_present_flag
    s.bit(false); // default display window
    s.bit(true); // vui timing info present
    s.u(32, u64::from(num_units));
    s.u(32, u64::from(time_scale));
    s.bit(false); // poc proportional
    if let Some((general, within, ed)) = hrd {
        s.bit(true); // vui_hrd_parameters_present_flag
        // hrd_parameters(1, 0): common info
        s.bit(true); // nal_hrd_parameters_present_flag
        s.bit(false); // vcl_hrd_parameters_present_flag
        // nal||vcl true:
        s.bit(false); // sub_pic_hrd_params_present_flag
        s.u(4, 0); // bit_rate_scale
        s.u(4, 0); // cpb_size_scale
        s.u(5, 0); // initial_cpb_removal_delay_length_minus1
        s.u(5, 0); // au_cpb_removal_delay_length_minus1
        s.u(5, 0); // dpb_output_delay_length_minus1
        // i=0
        s.bit(general); // fixed_pic_rate_general_flag
        if !general {
            s.bit(within); // fixed_pic_rate_within_cvs_flag
        }
        if general || within {
            s.ue(u64::from(ed)); // elemental_duration_in_tc_minus1
        } else {
            s.bit(false); // low_delay_hrd_flag
            s.ue(0); // cpb_cnt_minus1
        }
        // nal present => sub_layer_hrd_parameters(0): CpbCnt=1
        s.ue(0); // bit_rate_value_minus1
        s.ue(0); // cpb_size_value_minus1
        s.bit(false); // cbr_flag
    } else {
        s.bit(false); // vui_hrd_parameters_present_flag
    }
    s.bit(false); // bitstream_restriction_flag
    s.bit(true); // rbsp stop
    nal.extend(s.finish());
    nal
}

fn hevc_cfg_for_sps(sps: Vec<u8>) -> CodecConfig {
    CodecConfig::Hevc {
        config: isobmff::hevc_config::HEVCConfigurationBox::new(
            isobmff::hevc_config::HEVCDecoderConfigurationRecord {
                configuration_version: 1,
                general_profile_space: 0,
                general_tier_flag: false,
                general_profile_idc: 1,
                general_profile_compatibility_flags: 0,
                general_constraint_indicator_flags: 0,
                general_level_idc: 93,
                min_spatial_segmentation_idc: 0,
                parallelism_type: 0,
                chroma_format_idc: 1,
                bit_depth_luma_minus8: 0,
                bit_depth_chroma_minus8: 0,
                avg_frame_rate: 0,
                constant_frame_rate: 0,
                num_temporal_layers: 1,
                temporal_id_nested: true,
                length_size_minus_one: 3,
                arrays: vec![isobmff::nalu_types::HevcNalArray {
                    array_completeness: true,
                    reserved: false,
                    nal_unit_type: 33,
                    nalus: vec![isobmff::nalu_types::HevcNalUnit(sps)],
                }],
            },
        ),
        width: 320,
        height: 240,
    }
}

#[test]
fn hevc_vui_hrd_cases() {
    use FrameRateBasis::Normative;
    type Hrd = Option<(bool, bool, u32)>;
    struct Case {
        name: &'static str,
        field_seq: bool,
        ffip: bool,
        num_units: u32,
        hrd: Hrd,
        fixed_pic_rate: Option<Option<u32>>,
        field_seq_out: Option<bool>,
        rate: Option<Option<(u32, FrameRateBasis)>>,
    }
    let cases = [
        Case {
            name: "ed_one_halves_rate",
            field_seq: false,
            ffip: false,
            num_units: 1,
            hrd: Some((true, true, 1)),
            fixed_pic_rate: Some(Some(2)),
            field_seq_out: None,
            rate: Some(Some((12500, Normative))),
        },
        Case {
            name: "field_ed_one_quarters",
            field_seq: true,
            ffip: true,
            num_units: 1,
            hrd: Some((true, true, 1)),
            fixed_pic_rate: Some(Some(2)),
            field_seq_out: Some(true),
            rate: None,
        },
        Case {
            name: "ffip_one_field_seq_zero_none",
            field_seq: false,
            ffip: true,
            num_units: 1,
            hrd: Some((true, true, 0)),
            fixed_pic_rate: None,
            field_seq_out: None,
            rate: Some(None),
        },
        Case {
            name: "general_zero_within_one_tier_b_or_a",
            field_seq: false,
            ffip: false,
            num_units: 1,
            hrd: Some((false, true, 0)),
            fixed_pic_rate: Some(Some(1)),
            field_seq_out: None,
            rate: None,
        },
        Case {
            name: "general_zero_within_zero_tier_b",
            field_seq: false,
            ffip: false,
            num_units: 1,
            hrd: Some((false, false, 0)),
            fixed_pic_rate: Some(None),
            field_seq_out: None,
            rate: None,
        },
        Case {
            name: "field_seq_one_no_hrd_none",
            field_seq: true,
            ffip: true,
            num_units: 1,
            hrd: None,
            fixed_pic_rate: None,
            field_seq_out: None,
            rate: Some(None),
        },
        Case {
            name: "zero_num_units_none",
            field_seq: false,
            ffip: false,
            num_units: 0,
            hrd: Some((true, true, 0)),
            fixed_pic_rate: None,
            field_seq_out: None,
            rate: Some(None),
        },
    ];
    for c in cases {
        let sps = hevc_sps_vui_hrd(c.field_seq, c.ffip, c.num_units, 25, c.hrd);
        if let Some(expected) = c.fixed_pic_rate {
            let t = broadcast_bitstream::sps::decode_hevc_vui_timing(&sps)
                .unwrap()
                .unwrap();
            assert_eq!(t.hrd_fixed_pic_rate(), expected, "{}", c.name);
            if let Some(fs) = c.field_seq_out {
                assert_eq!(t.field_seq(), fs, "{}", c.name);
            }
        }
        if let Some(expected) = c.rate {
            assert_eq!(
                nominal_frame_rate(&hevc_cfg_for_sps(sps)),
                expected,
                "{}",
                c.name
            );
        }
    }
}

#[test]
fn hevc_field_seq_one_ffip_zero_no_hrd_is_none() {
    let sps = hevc_sps_vui_hrd(true, false, 1, 25, None);
    let cfg = CodecConfig::Hevc {
        config: isobmff::hevc_config::HEVCConfigurationBox::new(
            isobmff::hevc_config::HEVCDecoderConfigurationRecord {
                configuration_version: 1,
                general_profile_space: 0,
                general_tier_flag: false,
                general_profile_idc: 1,
                general_profile_compatibility_flags: 0,
                general_constraint_indicator_flags: 0,
                general_level_idc: 93,
                min_spatial_segmentation_idc: 0,
                parallelism_type: 0,
                chroma_format_idc: 1,
                bit_depth_luma_minus8: 0,
                bit_depth_chroma_minus8: 0,
                avg_frame_rate: 0,
                constant_frame_rate: 0,
                num_temporal_layers: 1,
                temporal_id_nested: true,
                length_size_minus_one: 3,
                arrays: vec![isobmff::nalu_types::HevcNalArray {
                    array_completeness: true,
                    reserved: false,
                    nal_unit_type: 33,
                    nalus: vec![isobmff::nalu_types::HevcNalUnit(sps)],
                }],
            },
        ),
        width: 320,
        height: 240,
    };
    assert_eq!(nominal_frame_rate(&cfg), None);
}
