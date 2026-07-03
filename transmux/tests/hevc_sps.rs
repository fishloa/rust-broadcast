//! Real-fixture HEVC SPS decode gate test (#516).
//!
//! Extracts the SPS NAL from the `hvcC` box of the committed `hevc_frag.mp4`
//! fixture (moov → trak → mdia → minf → stbl → stsd → hvc1 → hvcC), then
//! calls `transmux::decode_hevc_sps` directly and asserts the exact coded
//! dimensions / profile / chroma / bit-depth reported by `ffprobe`.
//!
//! ffprobe ground truth for `fixtures/transmux/hevc_frag.mp4`:
//!   codec_name=hevc  profile=Main  width=320  height=240
//!   pix_fmt=yuv420p  level=60

use broadcast_common::Parse;
use transmux::{decode_hevc_sps, rfc6381_hvc1, HEVCDecoderConfigurationRecord};

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

    // VisualSampleEntry has 78 bytes of fixed fields (ISO/IEC 14496-12 §12.1.3):
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

/// Decode the HEVC SPS from the real `hevc_frag.mp4` fixture via the
/// `hvcC` decoder configuration record and assert the exact oracle values
/// reported by ffprobe.
///
/// ffprobe oracle: 320×240, Main profile (idc=1), 4:2:0 (chroma=1), 8-bit,
/// level_idc=60, main tier (tier_flag=false).
#[test]
fn hevc_sps_decode_from_hvcc_fixture() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/hevc_frag.mp4"
    );
    let file = std::fs::read(path).expect("fixture hevc_frag.mp4 must exist");

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
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/hevc_frag.mp4"
    );
    let file = std::fs::read(path).expect("fixture hevc_frag.mp4 must exist");

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
        result.is_err(),
        "decode_hevc_sps on a truncated NAL must return Err, got: {result:?}"
    );
}

/// A one-byte input (shorter than the 2-byte HEVC NAL header) must also return Err.
#[test]
fn hevc_sps_too_short_returns_error() {
    let result = decode_hevc_sps(&[0x42]);
    assert!(
        result.is_err(),
        "one-byte input must return Err (need at least 2 bytes for HEVC NAL header)"
    );
}
