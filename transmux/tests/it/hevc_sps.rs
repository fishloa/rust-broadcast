//! Real-fixture HEVC SPS decode gate test (#516).
//!
//! Extracts the SPS NAL from the `hvcC` box of the committed `hevc-frag.mp4`
//! fixture (moov → trak → mdia → minf → stbl → stsd → hvc1 → hvcC), then
//! calls `transmux::decode_hevc_sps` directly and asserts the exact coded
//! dimensions / profile / chroma / bit-depth reported by `ffprobe`.
//!
//! ffprobe ground truth for `fixtures/mp4/hevc-frag.mp4`:
//!   codec_name=hevc  profile=Main  width=320  height=240
//!   pix_fmt=yuv420p  level=60

use broadcast_common::Parse;
use transmux::{HEVCDecoderConfigurationRecord, decode_hevc_sps, rfc6381_hvc1};

/// SPS `nal_unit_type` for H.265 (ITU-T H.265 Table 7-1).
const HEVC_SPS_NUT: u8 = 33;

// ---------------------------------------------------------------------------
// Box-walk helper (same technique as avc_hvc_config.rs)
// ---------------------------------------------------------------------------

/// Walk a flat sequence of ISO BMFF boxes and return the body of the first
/// box matching `four_cc`.  Returns `None` when not found.
fn find_box<'a>(data: &'a [u8], four_cc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        if size < 8 {
            break;
        }
        let ty = &data[offset + 4..offset + 8];
        let body = &data[offset + 8..offset + size];
        if ty == four_cc {
            return Some(body);
        }
        offset += size;
    }
    None
}

/// Extract the raw body of the `hvcC` box from a fragmented MP4 file by
/// navigating: moov → trak → mdia → minf → stbl → stsd → hvc1 → hvcC.
fn extract_hvcc_body(file: &[u8]) -> Vec<u8> {
    let moov = find_box(file, b"moov").expect("moov");
    let trak = find_box(moov, b"trak").expect("trak");
    let mdia = find_box(trak, b"mdia").expect("mdia");
    let minf = find_box(mdia, b"minf").expect("minf");
    let stbl = find_box(minf, b"stbl").expect("stbl");

    // stsd is a FullBox: first 8 bytes = box header (already stripped by find_box),
    // then version(1) + flags(3) + entry_count(4) = 8 more bytes before entries.
    let stsd_body = find_box(stbl, b"stsd").expect("stsd");
    // Skip version+flags (4 bytes) + entry_count (4 bytes).
    let entries = &stsd_body[8..];

    // Find the hvc1 sample entry.
    let hvc1_body = find_box(entries, b"hvc1").expect("hvc1");

    // VisualSampleEntry has 78 bytes of fixed fields (ISO/IEC 14496-12:2026 §12.1.3):
    //   reserved[6] + data_reference_index[2] = 8
    //   pre_defined[2] + reserved[2] + pre_defined[3][12] = 16
    //   width[2] + height[2] = 4
    //   horizresolution[4] + vertresolution[4] = 8
    //   reserved[4] = 4
    //   frame_count[2] = 2
    //   compressorname[32] = 32
    //   depth[2] = 2
    //   pre_defined[2] = 2
    //   Total = 8+16+4+8+4+2+32+2+2 = 78 bytes before optional child boxes.
    let config_region = &hvc1_body[78..];

    find_box(config_region, b"hvcC").expect("hvcC").to_vec()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Decode the HEVC SPS from the real `hevc-frag.mp4` fixture via the
/// `hvcC` decoder configuration record and assert the exact oracle values
/// reported by ffprobe.
///
/// ffprobe oracle: 320×240, Main profile (idc=1), 4:2:0 (chroma=1), 8-bit,
/// level_idc=60, main tier (tier_flag=false).
#[test]
fn hevc_sps_decode_from_hvcc_fixture() {
    let path = test_fixtures::path("mp4/hevc-frag.mp4");
    let file = std::fs::read(path).expect("fixture hevc-frag.mp4 must exist");

    // Extract hvcC body and parse the decoder config record.
    let hvcc_body = extract_hvcc_body(&file);
    let record =
        HEVCDecoderConfigurationRecord::parse(&hvcc_body).expect("hvcC parse must succeed");

    // Locate the SPS NAL array (nal_unit_type == 33).
    let sps_array = record
        .arrays
        .iter()
        .find(|a| a.nal_unit_type == HEVC_SPS_NUT)
        .expect("hvcC must contain an SPS NAL array");
    assert!(
        !sps_array.nalus.is_empty(),
        "SPS NAL array must have at least one entry"
    );

    // Call decode_hevc_sps directly on the raw SPS NAL bytes.
    let sps_nal = &sps_array.nalus[0].0;
    let info =
        decode_hevc_sps(sps_nal).expect("decode_hevc_sps must succeed on the hvcC fixture SPS");

    // --- Oracle assertions from ffprobe (profile=Main, width=320, height=240,
    // pix_fmt=yuv420p, level=60) ---

    // Main profile has general_profile_idc == 1.
    assert_eq!(info.general_profile_idc, 1, "Main profile (idc=1)");
    // Main tier: general_tier_flag == false.
    assert!(!info.general_tier_flag, "main tier");
    // ffprobe reports level=60 → general_level_idc == 60.
    assert_eq!(info.general_level_idc, 60, "level_idc 60");

    // yuv420p → chroma_format_idc == 1 (4:2:0, ITU-T H.265 Table 6-1).
    assert_eq!(info.chroma_format_idc, 1, "4:2:0 chroma");

    // 8-bit luma and chroma.
    assert_eq!(info.bit_depth_luma, 8, "8-bit luma");
    assert_eq!(info.bit_depth_chroma, 8, "8-bit chroma");

    // Coded dimensions after conformance-window crop must match ffprobe.
    assert_eq!(info.width, 320, "coded width 320");
    assert_eq!(info.height, 240, "coded height 240");

    // RFC 6381 codec string must be well-formed.
    let rfc = rfc6381_hvc1(&info);
    assert!(
        rfc.starts_with("hvc1."),
        "RFC 6381 string starts with hvc1."
    );
    assert!(rfc.len() > 5, "RFC 6381 string is non-trivial");
}

/// A test that bites: mutating the width/height assertion fails if the
/// decoder returns wrong dimensions, ensuring the test cannot pass by
/// accident or by a raw-passthrough implementation.
#[test]
fn hevc_sps_wrong_dimension_is_caught() {
    let path = test_fixtures::path("mp4/hevc-frag.mp4");
    let file = std::fs::read(path).expect("fixture hevc-frag.mp4 must exist");

    let hvcc_body = extract_hvcc_body(&file);
    let record =
        HEVCDecoderConfigurationRecord::parse(&hvcc_body).expect("hvcC parse must succeed");

    let sps_array = record
        .arrays
        .iter()
        .find(|a| a.nal_unit_type == HEVC_SPS_NUT)
        .expect("SPS array");

    let sps_nal = &sps_array.nalus[0].0;
    let info = decode_hevc_sps(sps_nal).expect("decode must succeed");

    // Sanity check: if a hypothetical implementation returned (0, 0) or any
    // value other than (320, 240), the oracle test above would already catch
    // it.  Here we verify the decoded values are exactly the ffprobe values
    // and NOT accidentally zero or swapped.
    assert_ne!(info.width, 0, "width must not be zero");
    assert_ne!(info.height, 0, "height must not be zero");
    assert_ne!(
        info.width, info.height,
        "width != height (not a square, not swapped)"
    );
    assert_eq!((info.width, info.height), (320, 240), "exact oracle");
}

/// Truncated SPS NAL (first 3 bytes only) must return Err, not a bogus decode.
#[test]
fn hevc_sps_truncated_returns_error() {
    // A real HEVC SPS NAL header begins with 0x42 0x01 (nal_unit_type=33, nuh_layer_id=0,
    // nuh_temporal_id_plus1=1).  Three bytes is far too short to decode.
    let truncated: &[u8] = &[0x42, 0x01, 0x00];
    let result = decode_hevc_sps(truncated);
    assert!(
        matches!(result, Err(transmux::error::Error::BufferTooShort(..))),
        "decode_hevc_sps on a truncated NAL must return Err, got: {result:?}"
    );
}

/// A one-byte input (shorter than the 2-byte HEVC NAL header) must also return Err.
#[test]
fn hevc_sps_too_short_returns_error() {
    let result = decode_hevc_sps(&[0x42]);
    assert!(
        matches!(result, Err(transmux::error::Error::BufferTooShort(..))),
        "one-byte input must return Err (need at least 2 bytes for HEVC NAL header)"
    );
}

// ---------------------------------------------------------------------------
// Synthetic SPS bounds (audit 23 #3849, #3851, #3852, #3853, #3854, #3855, #3856)
// ---------------------------------------------------------------------------

use crate::sps_builder::{HevcHrd, HevcHrdLayer, HevcRps, HevcSps, HevcVui, hevc_sps};
use transmux::sps::decode_hevc_interlace_sps;

const PROGRESSIVE_SOURCE: u64 = 1 << 47;
const INTERLACED_SOURCE: u64 = 1 << 46;

#[test]
fn hevc_frame_field_info_inferred_only_when_both_source_flags_set() {
    for (flags, expected) in [
        (0, false),
        (PROGRESSIVE_SOURCE, false),
        (INTERLACED_SOURCE, false),
        (PROGRESSIVE_SOURCE | INTERLACED_SOURCE, true),
    ] {
        let nal = hevc_sps(&HevcSps {
            constraint_flags: flags,
            ..HevcSps::default()
        });
        let interlace = decode_hevc_interlace_sps(&nal).expect("decode interlace facts");
        assert_eq!(
            interlace.frame_field_info_present, expected,
            "constraint flags {flags:#x}: inference requires both source flags"
        );
    }
}

fn timing_vui(n: u32, ts: u32) -> HevcVui {
    HevcVui {
        field_seq: false,
        frame_field_info_present: false,
        default_display: None,
        timing: Some((n, ts)),
        hrd: None,
    }
}

#[test]
fn hevc_vui_timing_frame_rate_and_zero_num_units() {
    let nal = hevc_sps(&HevcSps {
        vui: Some(timing_vui(1001, 60000)),
        ..HevcSps::default()
    });
    let info = transmux::decode_hevc_sps(&nal).expect("decode SPS");
    assert_eq!(info.num_units_in_tick, Some(1001));
    assert_eq!(info.time_scale, Some(60000));
    let fps = info.fps.expect("frame rate");
    assert!(
        (fps - 59.94).abs() < 1e-3,
        "HEVC fps = time_scale / num_units_in_tick, got {fps}"
    );

    let nal = hevc_sps(&HevcSps {
        vui: Some(timing_vui(0, 60000)),
        ..HevcSps::default()
    });
    let info = transmux::decode_hevc_sps(&nal).expect("decode SPS");
    assert_eq!(info.num_units_in_tick, Some(0));
    assert_eq!(info.time_scale, Some(60000));
    assert_eq!(info.fps, None, "num_units_in_tick 0 yields no frame rate");
}

#[test]
fn hevc_hrd_walk_reads_sub_layer_hrd_and_reports_last_fixed_rate() {
    let nal = hevc_sps(&HevcSps {
        max_sub_layers_minus1: 1,
        sub_layer_profile_present: vec![false, false],
        sub_layer_level_present: vec![false, false],
        vui: Some(HevcVui {
            field_seq: false,
            frame_field_info_present: false,
            default_display: None,
            timing: Some((1, 50)),
            hrd: Some(HevcHrd {
                nal: true,
                vcl: false,
                sub_pic: true,
                layers: vec![
                    HevcHrdLayer {
                        general: false,
                        within: false,
                        low_delay: false,
                        cpb_minus1: 2,
                        elemental: 0,
                    },
                    HevcHrdLayer {
                        general: true,
                        within: true,
                        low_delay: false,
                        cpb_minus1: 0,
                        elemental: 4,
                    },
                ],
            }),
        }),
        ..HevcSps::default()
    });
    let timing = transmux::decode_hevc_vui_timing(&nal)
        .expect("decode")
        .expect("timing present");
    assert_eq!((timing.num_units_in_tick, timing.time_scale), (1, 50));
    assert_eq!(
        timing.hrd_fixed_pic_rate,
        Some(5),
        "HighestTid elemental_duration_in_tc_minus1 + 1"
    );
}

fn hrd_cpb_sps(cpb_minus1: u64) -> Vec<u8> {
    hevc_sps(&HevcSps {
        vui: Some(HevcVui {
            field_seq: false,
            frame_field_info_present: false,
            default_display: None,
            timing: Some((1, 50)),
            hrd: Some(HevcHrd {
                nal: true,
                vcl: false,
                sub_pic: true,
                layers: vec![HevcHrdLayer {
                    general: false,
                    within: false,
                    low_delay: false,
                    cpb_minus1,
                    elemental: 0,
                }],
            }),
        }),
        ..HevcSps::default()
    })
}

#[test]
fn hevc_hrd_cpb_cnt_bound() {
    let timing = transmux::decode_hevc_vui_timing(&hrd_cpb_sps(31))
        .expect("decode")
        .expect("31 is the legal maximum");
    assert_eq!((timing.num_units_in_tick, timing.time_scale), (1, 50));

    assert!(
        transmux::decode_hevc_vui_timing(&hrd_cpb_sps(32))
            .expect("decode")
            .is_none(),
        "cpb_cnt_minus1 32 is outside 0..=31, so the walk stops before timing"
    );
}

#[test]
fn hevc_scaling_list_data_walk_reaches_vui_timing() {
    let mut modes = Vec::new();
    for size_id in 0..4u32 {
        let count = if size_id == 3 { 2 } else { 6 };
        for m in 0..count {
            modes.push(m % 2 == 1);
        }
    }
    let nal = hevc_sps(&HevcSps {
        scaling_list: Some(modes),
        vui: Some(timing_vui(1, 50)),
        ..HevcSps::default()
    });
    let timing = transmux::decode_hevc_vui_timing(&nal)
        .expect("decode")
        .expect("timing after scaling_list_data");
    assert_eq!((timing.num_units_in_tick, timing.time_scale), (1, 50));
}

fn explicit_zero_pairs(count: usize) -> Vec<HevcRps> {
    (0..count)
        .map(|_| HevcRps::explicit(Vec::new(), Vec::new()))
        .collect()
}

#[test]
fn hevc_num_short_term_ref_pic_sets_bound() {
    let nal = hevc_sps(&HevcSps {
        rps: explicit_zero_pairs(64),
        vui: Some(timing_vui(1, 50)),
        ..HevcSps::default()
    });
    assert_eq!(
        transmux::decode_hevc_vui_timing(&nal)
            .expect("decode")
            .map(|t| (t.num_units_in_tick, t.time_scale)),
        Some((1, 50)),
        "64 short-term RPS is the legal maximum"
    );

    let nal = hevc_sps(&HevcSps {
        rps: explicit_zero_pairs(65),
        vui: Some(timing_vui(1, 50)),
        ..HevcSps::default()
    });
    assert!(
        transmux::decode_hevc_vui_timing(&nal)
            .expect("decode")
            .is_none(),
        "65 short-term RPS is outside 0..=64"
    );
}

fn predicted(
    delta_rps_sign: bool,
    abs_delta_rps_minus1: u64,
    used: Vec<bool>,
    use_delta: Vec<bool>,
) -> HevcRps {
    HevcRps {
        inter: true,
        deltas_neg: Vec::new(),
        deltas_pos: Vec::new(),
        delta_rps_sign,
        abs_delta_rps_minus1,
        used,
        use_delta,
    }
}

#[test]
fn hevc_inter_rps_derivation_reaches_vui_timing() {
    let rps = vec![
        HevcRps::explicit(vec![0, 1], vec![0]),
        predicted(false, 2, vec![true, true, true, true], vec![false; 4]),
        predicted(
            true,
            1,
            vec![true, true, false, true],
            vec![false, false, false, false],
        ),
        predicted(false, 0, vec![true, true, false], vec![false, false, false]),
        predicted(false, 0, vec![true, true, true], vec![false, false, false]),
    ];
    let nal = hevc_sps(&HevcSps {
        rps,
        vui: Some(timing_vui(1, 50)),
        ..HevcSps::default()
    });
    let timing = transmux::decode_hevc_vui_timing(&nal)
        .expect("decode")
        .expect("timing after inter-RPS derivation");
    assert_eq!((timing.num_units_in_tick, timing.time_scale), (1, 50));
}

#[test]
fn hevc_explicit_rps_picture_count_bound() {
    let ok = |rps: Vec<HevcRps>| {
        hevc_sps(&HevcSps {
            rps,
            vui: Some(timing_vui(1, 50)),
            ..HevcSps::default()
        })
    };
    let decodes = |nal: &[u8]| {
        transmux::decode_hevc_vui_timing(nal)
            .expect("decode")
            .map(|t| (t.num_units_in_tick, t.time_scale))
    };

    assert_eq!(
        decodes(&ok(vec![HevcRps::explicit(vec![0; 16], Vec::new())])),
        Some((1, 50)),
        "16 negative pictures is inside the bound"
    );
    assert_eq!(
        decodes(&ok(vec![HevcRps::explicit(Vec::new(), vec![0; 16])])),
        Some((1, 50)),
        "16 positive pictures is inside the bound"
    );
    assert_eq!(
        decodes(&ok(vec![HevcRps::explicit(vec![0; 17], Vec::new())])),
        None,
        "17 negative pictures is outside 0..=16"
    );
    assert_eq!(
        decodes(&ok(vec![HevcRps::explicit(Vec::new(), vec![0; 17])])),
        None,
        "17 positive pictures is outside 0..=16"
    );
}

#[test]
fn hevc_derived_rps_growing_past_sixteen_is_rejected() {
    let nal = hevc_sps(&HevcSps {
        rps: vec![
            HevcRps::explicit(vec![0; 8], vec![0; 8]),
            predicted(false, 9, vec![true; 17], vec![false; 17]),
        ],
        vui: Some(timing_vui(1, 50)),
        ..HevcSps::default()
    });
    assert!(
        transmux::decode_hevc_vui_timing(&nal)
            .expect("decode")
            .is_none(),
        "a derived set with 17 entries stops the walk before timing"
    );
}

#[test]
fn hevc_explicit_rps_delta_accumulation_feeds_prediction() {
    let nal = hevc_sps(&HevcSps {
        rps: vec![
            HevcRps::explicit(Vec::new(), vec![1, 1]),
            predicted(true, 1, vec![true, true, true], vec![false; 3]),
            predicted(false, 0, vec![true, true, true], vec![false; 3]),
            predicted(false, 0, vec![true, true, true, true], vec![false; 4]),
        ],
        vui: Some(timing_vui(1, 50)),
        ..HevcSps::default()
    });
    let timing = transmux::decode_hevc_vui_timing(&nal)
        .expect("decode")
        .expect("timing after prediction from the accumulated positive POCs");
    assert_eq!((timing.num_units_in_tick, timing.time_scale), (1, 50));
}

#[test]
fn hevc_long_term_ref_poc_width_follows_log2_max_poc_lsb() {
    let nal = hevc_sps(&HevcSps {
        log2_max_poc_lsb_minus4: 4,
        long_term_ref_pics: Some(2),
        vui: Some(timing_vui(1, 50)),
        ..HevcSps::default()
    });
    let timing = transmux::decode_hevc_vui_timing(&nal)
        .expect("decode")
        .expect("timing after long-term ref entries");
    assert_eq!((timing.num_units_in_tick, timing.time_scale), (1, 50));
}

#[test]
fn rfc6381_hvc1_emits_each_constraint_byte() {
    let nal = hevc_sps(&HevcSps {
        constraint_flags: 0x91_A2_B3_C4_D5_E6,
        ..HevcSps::default()
    });
    let info = transmux::decode_hevc_sps(&nal).expect("decode SPS");
    assert_eq!(info.general_constraint_indicator_flags, 0x91_A2_B3_C4_D5_E6);
    assert_eq!(
        rfc6381_hvc1(&info),
        "hvc1.1.0.L93.91.A2.B3.C4.D5.E6",
        "every nonzero constraint byte is emitted in order"
    );
}

#[test]
fn hevc_multi_sub_layer_ptl_reserved_padding_is_skipped() {
    let nal = hevc_sps(&HevcSps {
        max_sub_layers_minus1: 2,
        sub_layer_profile_present: vec![true, false],
        sub_layer_level_present: vec![false, true],
        width: 640,
        height: 360,
        ..HevcSps::default()
    });
    let info = transmux::decode_hevc_sps(&nal).expect("decode SPS");
    assert_eq!(info.width, 640);
    assert_eq!(info.height, 360);
    assert_eq!(info.general_profile_idc, 1);
    assert_eq!(info.general_level_idc, 93);
}

#[test]
fn hevc_inter_rps_negative_offsets_from_an_empty_negative_set() {
    let nal = hevc_sps(&HevcSps {
        rps: vec![
            HevcRps::explicit(Vec::new(), vec![0, 0]),
            predicted(true, 6, vec![true, true, true], vec![false; 3]),
        ],
        vui: Some(timing_vui(1, 50)),
        ..HevcSps::default()
    });
    let timing = transmux::decode_hevc_vui_timing(&nal)
        .expect("decode")
        .expect("timing after (7-61) over a non-empty positive set");
    assert_eq!((timing.num_units_in_tick, timing.time_scale), (1, 50));
}
