//! H.264 / H.265 sequence parameter set (SPS) decode.
//!
//! Decodes the fields needed by the transmux pipeline — profile / level /
//! dimensions / chroma / bit-depth — from the raw NAL unit bytes stored in
//! [`AvcSps`](crate::nalu_types::AvcSps) and [`HevcNalUnit`](crate::nalu_types::HevcNalUnit).
//!
//! # Spec citations
//!
//! - **H.264 SPS**: ITU-T H.264 §7.3.2.1.1 (sequence parameter set data).
//! - **H.265 SPS + PTL**: ITU-T H.265 §7.3.2.2 (SPS) + §7.3.3 (profile_tier_level).
//! - **Coded dimensions**: see `fixtures/ts/CODEC-ORACLE.md`.
//! - **Emulation prevention**: H.264 §7.3 bullet 1 (leading zero_byte),
//!   §7.4.1 NAL semantics (emulation_prevention_three_byte); H.265 §7.4.2.1.

use crate::bitreader::BitReader;
use crate::error::{Error, Result};
use alloc::string::String;

// ---------------------------------------------------------------------------
// H.264 helpers
// ---------------------------------------------------------------------------

/// Profiles that require the high-profile SPS branch (§7.3.2.1.1):
/// chroma_format_idc, bit_depth_*_minus8, and optionally scaling lists.
const H264_HIGH_PROFILES: &[u8] = &[100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135];

fn is_high_profile(profile_idc: u8) -> bool {
    H264_HIGH_PROFILES.contains(&profile_idc)
}

/// Computed SubWidthC / SubHeightC from `chroma_format_idc`
/// (ITU-T H.264 Table 6-1).
fn sub_width_c(chroma_format_idc: u8) -> u32 {
    match chroma_format_idc {
        0 => 1,
        1 => 2,
        2 => 2,
        _ => 1,
    }
}

fn sub_height_c(chroma_format_idc: u8) -> u32 {
    match chroma_format_idc {
        0 => 1,
        1 => 2,
        2 => 1,
        _ => 1,
    }
}

// ---------------------------------------------------------------------------
// AvcSpsInfo
// ---------------------------------------------------------------------------

/// Decoded fields from an H.264/AVC sequence parameter set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvcSpsInfo {
    /// `profile_idc` — SPS byte index 1.
    pub profile_idc: u8,
    /// The constraint-flags byte (SPS byte index 2 verbatim):
    /// constraint_set0..5 `[7:2]` + reserved_zero_2bits `[1:0]`.
    pub constraint_flags: u8,
    /// `level_idc` — SPS byte index 3.
    pub level_idc: u8,
    /// `chroma_format_idc` (defaults to 1 for non-high profiles).
    pub chroma_format_idc: u8,
    /// `separate_colour_plane_flag`
    pub separate_colour_plane: bool,
    /// `bit_depth_luma` (= `bit_depth_luma_minus8 + 8`, default 8).
    pub bit_depth_luma: u8,
    /// `bit_depth_chroma` (= `bit_depth_chroma_minus8 + 8`, default 8).
    pub bit_depth_chroma: u8,
    /// `frame_mbs_only_flag`
    pub frame_mbs_only: bool,
    /// Coded width in luma samples (after frame cropping).
    pub width: u32,
    /// Coded height in luma samples (after frame cropping).
    pub height: u32,
}

/// Decode an H.264 SPS RBSP.
///
/// `sps_bytes` is the raw NAL unit including the 1-byte NAL header
/// (as returned by [`iter_annexb_nals`](crate::iter_annexb_nals)).
/// The NAL header byte is skipped; decoding starts at the next byte.
pub fn decode_avc_sps(sps_bytes: &[u8]) -> Result<AvcSpsInfo> {
    if sps_bytes.len() < 4 {
        return Err(Error::BufferTooShort {
            need: 4,
            have: sps_bytes.len(),
            what: "H.264 SPS header",
        });
    }

    // Byte 0 is the NAL header (nal_ref_idc + nal_unit_type).
    // Byte 1 is profile_idc.
    let profile_idc = sps_bytes[1];
    let constraint_flags = sps_bytes[2];
    let level_idc = sps_bytes[3];

    // Unescape the RBSP: starts after level_idc (byte index 3, i.e. sps_bytes[4]).
    let mut r = BitReader::with_unescape(&sps_bytes[4..], "H.264 SPS")?;

    // seq_parameter_set_id — ue(v)
    let _sps_id = r.read_ue("seq_parameter_set_id")?;

    // High-profile branch
    let (chroma_format_idc, separate_colour_plane, bit_depth_luma, bit_depth_chroma) =
        if is_high_profile(profile_idc) {
            let chroma_format_idc = r.read_ue("chroma_format_idc")? as u8;
            let separate_colour_plane = if chroma_format_idc == 3 {
                r.read_flag("separate_colour_plane_flag")?
            } else {
                false
            };
            let bit_depth_luma_minus8 = r.read_ue("bit_depth_luma_minus8")? as u8;
            let bit_depth_chroma_minus8 = r.read_ue("bit_depth_chroma_minus8")? as u8;
            let _ = r.read_flag("qpprime_y_zero_transform_bypass_flag")?;
            let scaling_matrix_present = r.read_flag("seq_scaling_matrix_present_flag")?;
            if scaling_matrix_present {
                let list_count = if chroma_format_idc != 3 { 8 } else { 12 };
                for _i in 0..list_count {
                    let list_present = r.read_flag("seq_scaling_list_present_flag[i]")?;
                    if list_present {
                        let size: usize = if _i < 6 { 16 } else { 64 };
                        let mut last_scale: i64 = 8;
                        let mut next_scale: i64 = 8;
                        for _j in 0..size {
                            if next_scale != 0 {
                                let delta_scale = r.read_se("delta_scale")?;
                                next_scale = (last_scale + delta_scale + 256) % 256;
                            }
                            if next_scale == 0 {
                                // use default; don't change last_scale
                            } else {
                                last_scale = next_scale;
                            }
                        }
                    }
                }
            }
            (
                chroma_format_idc,
                separate_colour_plane,
                bit_depth_luma_minus8 + 8,
                bit_depth_chroma_minus8 + 8,
            )
        } else {
            (1, false, 8, 8)
        };

    // log2_max_frame_num_minus4 — ue(v)
    let _ = r.read_ue("log2_max_frame_num_minus4")?;
    // pic_order_cnt_type — ue(v)
    let pic_order_cnt_type = r.read_ue("pic_order_cnt_type")?;
    if pic_order_cnt_type == 0 {
        let _ = r.read_ue("log2_max_pic_order_cnt_lsb_minus4")?;
    } else if pic_order_cnt_type == 1 {
        let _ = r.read_flag("delta_pic_order_always_zero_flag")?;
        let _ = r.read_se("offset_for_non_ref_pic")?;
        let _ = r.read_se("offset_for_top_to_bottom_field")?;
        let num_ref_frames = r.read_ue("num_ref_frames_in_pic_order_cnt_cycle")?;
        for _ in 0..num_ref_frames {
            let _ = r.read_se("offset_for_ref_frame[i]")?;
        }
    }
    // max_num_ref_frames — ue(v)
    let _ = r.read_ue("max_num_ref_frames")?;
    // gaps_in_frame_num_value_allowed_flag — 1 bit
    let _ = r.read_flag("gaps_in_frame_num_value_allowed_flag")?;
    // pic_width_in_mbs_minus1 — ue(v)
    let pic_width_in_mbs_minus1 = r.read_ue("pic_width_in_mbs_minus1")?;
    // pic_height_in_map_units_minus1 — ue(v)
    let pic_height_in_map_units_minus1 = r.read_ue("pic_height_in_map_units_minus1")?;
    // frame_mbs_only_flag — 1 bit
    let frame_mbs_only = r.read_flag("frame_mbs_only_flag")?;

    if !frame_mbs_only {
        let _ = r.read_flag("mb_adaptive_frame_field_flag")?;
    }

    // direct_8x8_inference_flag — 1 bit
    let _ = r.read_flag("direct_8x8_inference_flag")?;
    // frame_cropping_flag — 1 bit
    let frame_cropping = r.read_flag("frame_cropping_flag")?;

    let (crop_left, crop_right, crop_top, crop_bottom) = if frame_cropping {
        let cl = r.read_ue("frame_crop_left_offset")?;
        let cr = r.read_ue("frame_crop_right_offset")?;
        let ct = r.read_ue("frame_crop_top_offset")?;
        let cb = r.read_ue("frame_crop_bottom_offset")?;
        (cl, cr, ct, cb)
    } else {
        (0, 0, 0, 0)
    };

    // Compute coded dimensions
    let pic_width_in_mbs = pic_width_in_mbs_minus1 + 1;
    let pic_height_in_map_units = pic_height_in_map_units_minus1 + 1;
    let frame_height_in_mbs = (2 - frame_mbs_only as u64) * pic_height_in_map_units;

    let crop_unit_x = sub_width_c(chroma_format_idc) as u64;
    let crop_unit_y = sub_height_c(chroma_format_idc) as u64 * (2 - frame_mbs_only as u64);

    let width = (pic_width_in_mbs * 16).saturating_sub(crop_unit_x * (crop_left + crop_right));
    let height = (frame_height_in_mbs * 16).saturating_sub(crop_unit_y * (crop_top + crop_bottom));

    Ok(AvcSpsInfo {
        profile_idc,
        constraint_flags,
        level_idc,
        chroma_format_idc,
        separate_colour_plane,
        bit_depth_luma,
        bit_depth_chroma,
        frame_mbs_only,
        width: width as u32,
        height: height as u32,
    })
}

// ---------------------------------------------------------------------------
// HevcSpsInfo
// ---------------------------------------------------------------------------

/// Decoded fields from an H.265/HEVC sequence parameter set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HevcSpsInfo {
    /// `general_profile_space` (2 bits).
    pub general_profile_space: u8,
    /// `general_tier_flag` (1 bit).
    pub general_tier_flag: bool,
    /// `general_profile_idc` (5 bits).
    pub general_profile_idc: u8,
    /// `general_profile_compatibility_flags` (32 bits).
    pub general_profile_compatibility_flags: u32,
    /// `general_constraint_indicator_flags` (48 bits, i.e. 6 bytes).
    pub general_constraint_indicator_flags: u64,
    /// `general_level_idc` (8 bits).
    pub general_level_idc: u8,
    /// `chroma_format_idc` — ue(v)
    pub chroma_format_idc: u8,
    /// `bit_depth_luma` (= `bit_depth_luma_minus8 + 8`).
    pub bit_depth_luma: u8,
    /// `bit_depth_chroma` (= `bit_depth_chroma_minus8 + 8`).
    pub bit_depth_chroma: u8,
    /// Coded width in luma samples (conformance-window cropped).
    pub width: u32,
    /// Coded height in luma samples (conformance-window cropped).
    pub height: u32,
}

/// Decode an H.265 SPS RBSP (NAL unit body after the 2-byte NAL header).
///
/// `sps_bytes` is the full NAL unit including the 2-byte NAL header.
pub fn decode_hevc_sps(sps_bytes: &[u8]) -> Result<HevcSpsInfo> {
    if sps_bytes.len() < 2 {
        return Err(Error::BufferTooShort {
            need: 2,
            have: sps_bytes.len(),
            what: "HEVC SPS header",
        });
    }

    let mut r = BitReader::with_unescape(&sps_bytes[2..], "HEVC SPS")?;

    // sps_video_parameter_set_id — u(4)
    let _ = r.read_bits(4, "sps_video_parameter_set_id")?;
    // sps_max_sub_layers_minus1 — u(3)
    let sps_max_sub_layers_minus1 = r.read_bits(3, "sps_max_sub_layers_minus1")? as u8;
    // sps_temporal_id_nesting_flag — u(1)
    let _ = r.read_flag("sps_temporal_id_nesting_flag")?;

    // profile_tier_level(1, sps_max_sub_layers_minus1)
    let (
        general_profile_space,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility_flags,
        general_constraint_indicator_flags,
        general_level_idc,
    ) = decode_ptl(&mut r, true, sps_max_sub_layers_minus1)?;

    // sps_seq_parameter_set_id — ue(v)
    let _ = r.read_ue("sps_seq_parameter_set_id")?;
    // chroma_format_idc — ue(v)
    let chroma_format_idc = r.read_ue("chroma_format_idc")? as u8;
    if chroma_format_idc == 3 {
        let _ = r.read_flag("separate_colour_plane_flag")?;
    }
    // pic_width_in_luma_samples — ue(v)
    let pic_width_in_luma_samples = r.read_ue("pic_width_in_luma_samples")?;
    // pic_height_in_luma_samples — ue(v)
    let pic_height_in_luma_samples = r.read_ue("pic_height_in_luma_samples")?;
    // conformance_window_flag — u(1)
    let conformance_window = r.read_flag("conformance_window_flag")?;
    let (conf_win_left, conf_win_right, conf_win_top, conf_win_bottom) = if conformance_window {
        let left = r.read_ue("conf_win_left_offset")?;
        let right = r.read_ue("conf_win_right_offset")?;
        let top = r.read_ue("conf_win_top_offset")?;
        let bottom = r.read_ue("conf_win_bottom_offset")?;
        (left, right, top, bottom)
    } else {
        (0, 0, 0, 0)
    };

    // bit_depth_luma_minus8 — ue(v)
    let bit_depth_luma_minus8 = r.read_ue("bit_depth_luma_minus8")? as u8;
    // bit_depth_chroma_minus8 — ue(v)
    let bit_depth_chroma_minus8 = r.read_ue("bit_depth_chroma_minus8")? as u8;

    // Compute cropped dimensions.
    let sub_width_c = match chroma_format_idc {
        1 | 2 => 2,
        3 => 1,
        _ => 1,
    };
    let sub_height_c = match chroma_format_idc {
        1 => 2,
        2 | 3 => 1,
        _ => 1,
    };

    let width = pic_width_in_luma_samples
        .saturating_sub((sub_width_c as u64) * (conf_win_left + conf_win_right));
    let height = pic_height_in_luma_samples
        .saturating_sub((sub_height_c as u64) * (conf_win_top + conf_win_bottom));

    Ok(HevcSpsInfo {
        general_profile_space,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility_flags,
        general_constraint_indicator_flags,
        general_level_idc,
        chroma_format_idc,
        bit_depth_luma: bit_depth_luma_minus8 + 8,
        bit_depth_chroma: bit_depth_chroma_minus8 + 8,
        width: width as u32,
        height: height as u32,
    })
}

// ---------------------------------------------------------------------------
// RFC 6381 helpers
// ---------------------------------------------------------------------------

/// Build RFC 6381 `avc1.PPCCLL` codec string.
///
/// UPPERCASE hex: `avc1.<profile_idc><constraint_flags><level_idc>`,
/// each as two hex digits (e.g. `avc1.4D400D`).
pub fn rfc6381_avc1(profile_idc: u8, constraint_flags: u8, level_idc: u8) -> String {
    let mut s = String::with_capacity(12);
    s.push_str("avc1.");
    write_hex_byte(&mut s, profile_idc);
    write_hex_byte(&mut s, constraint_flags);
    write_hex_byte(&mut s, level_idc);
    s
}

fn write_hex_byte(s: &mut String, byte: u8) {
    let hi = (byte >> 4) & 0xF;
    let lo = byte & 0xF;
    s.push(hex_char(hi));
    s.push(hex_char(lo));
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

/// Build RFC 6381 `hvc1.…` codec string (H.265 §3.3).
///
/// Per RFC 6381 §3.3:
/// `hvc1.<profile_space><profile_idc>.<compat>.<tier><level>[.<constraint>…]`
pub fn rfc6381_hvc1(info: &HevcSpsInfo) -> String {
    let mut s = String::with_capacity(48);
    s.push_str("hvc1.");

    let profile_byte = (info.general_profile_space << 5) | info.general_profile_idc;
    write_hex_byte(&mut s, profile_byte);

    s.push('.');
    write_compat_flags(&mut s, info.general_profile_compatibility_flags);

    s.push('.');
    let tier_char = if info.general_tier_flag { 'H' } else { 'L' };
    s.push(tier_char);
    write_hex_byte(&mut s, info.general_level_idc);

    let constraints = info.general_constraint_indicator_flags;
    let bytes: [u8; 6] = [
        ((constraints >> 40) & 0xFF) as u8,
        ((constraints >> 32) & 0xFF) as u8,
        ((constraints >> 24) & 0xFF) as u8,
        ((constraints >> 16) & 0xFF) as u8,
        ((constraints >> 8) & 0xFF) as u8,
        (constraints & 0xFF) as u8,
    ];

    let last_nonzero = bytes.iter().rposition(|&b| b != 0);
    if let Some(end) = last_nonzero {
        for (i, &b) in bytes.iter().enumerate() {
            if i > end {
                break;
            }
            s.push('.');
            write_hex_byte(&mut s, b);
        }
    }

    s
}

/// Write the reversed-bit compatibility flags as hex, dropping trailing zeros.
fn write_compat_flags(s: &mut String, flags: u32) {
    let reversed = flags.reverse_bits();
    let mut hex = [0u8; 8];
    let mut v = reversed;
    for i in (0..8).rev() {
        let nib = (v & 0xF) as u8;
        hex[i] = if nib < 10 {
            b'0' + nib
        } else {
            b'A' + (nib - 10)
        };
        v >>= 4;
    }
    let end = (0..8)
        .rev()
        .find(|&i| hex[i] != b'0')
        .map(|i| i + 1)
        .unwrap_or(1);
    for &b in &hex[..end] {
        s.push(b as char);
    }
}

/// Build RFC 6381 `mp4a.40.<AOT>` codec string from AAC `AudioObjectType`.
pub fn rfc6381_mp4a(aot: u8) -> String {
    let mut s = String::with_capacity(16);
    s.push_str("mp4a.40.");
    write_decimal(&mut s, aot);
    s
}

fn write_decimal(s: &mut String, mut n: u8) {
    if n == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 4];
    let mut len = 0;
    while n > 0 {
        buf[len] = b'0' + (n % 10);
        n /= 10;
        len += 1;
    }
    for i in (0..len).rev() {
        s.push(buf[i] as char);
    }
}

// ---------------------------------------------------------------------------
// profile_tier_level decode (H.265 §7.3.3)
// ---------------------------------------------------------------------------

fn decode_ptl(
    r: &mut BitReader,
    profile_present_flag: bool,
    max_sub_layers_minus1: u8,
) -> Result<(u8, bool, u8, u32, u64, u8)> {
    let (general_profile_space, general_tier_flag, general_profile_idc) = if profile_present_flag {
        (
            r.read_bits(2, "general_profile_space")? as u8,
            r.read_flag("general_tier_flag")?,
            r.read_bits(5, "general_profile_idc")? as u8,
        )
    } else {
        (0, false, 0)
    };

    let general_profile_compatibility_flags =
        r.read_bits(32, "general_profile_compatibility_flags")? as u32;

    let general_constraint_indicator_flags =
        r.read_bits(48, "general_constraint_indicator_flags")?;

    let general_level_idc = r.read_bits(8, "general_level_idc")? as u8;

    let mut sub_layer_profile_present_flag = [false; 7];
    let mut sub_layer_level_present_flag = [false; 7];
    for i in 0..max_sub_layers_minus1 {
        sub_layer_profile_present_flag[i as usize] =
            r.read_flag("sub_layer_profile_present_flag[i]")?;
        sub_layer_level_present_flag[i as usize] =
            r.read_flag("sub_layer_level_present_flag[i]")?;
    }

    if max_sub_layers_minus1 > 0 {
        for _i in max_sub_layers_minus1..8 {
            let _ = r.read_bits(2, "reserved_zero_2bits")?;
        }
    }

    for i in 0..max_sub_layers_minus1 {
        if sub_layer_profile_present_flag[i as usize] {
            let _ = r.read_bits(2, "sub_layer_profile_space")?;
            let _ = r.read_flag("sub_layer_tier_flag")?;
            let _ = r.read_bits(5, "sub_layer_profile_idc")?;
            let _ = r.read_bits(32, "sub_layer_profile_compatibility_flags")?;
            let _ = r.read_bits(48, "sub_layer_constraint_indicator_flags")?;
        }
        if sub_layer_level_present_flag[i as usize] {
            let _ = r.read_bits(8, "sub_layer_level_idc")?;
        }
    }

    Ok((
        general_profile_space,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility_flags,
        general_constraint_indicator_flags,
        general_level_idc,
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6381_avc1_baseline() {
        assert_eq!(rfc6381_avc1(66, 0xC0, 13), "avc1.42C00D");
    }

    #[test]
    fn rfc6381_avc1_main() {
        assert_eq!(rfc6381_avc1(77, 0x40, 13), "avc1.4D400D");
    }

    #[test]
    fn rfc6381_avc1_high() {
        assert_eq!(rfc6381_avc1(100, 0x00, 13), "avc1.64000D");
    }

    #[test]
    fn rfc6381_mp4a_aac_lc() {
        assert_eq!(rfc6381_mp4a(2), "mp4a.40.2");
    }

    #[test]
    fn decode_high_sps_with_scaling_lists() {
        // Hand-built high-profile SPS with embedded scaling lists.
        //
        // Full NAL including header:
        //   [0] 0x67  NAL header (nal_ref_idc=3, nal_unit_type=7)
        //   [1] 0x64  profile_idc=100 (High)
        //   [2] 0x00  constraint_flags
        //   [3] 0x0D  level_idc=13
        //
        // Bytes [4..] are the bitstream computed by a Python script (see
        // test comment in source history for exact bit packing).

        let sps_nal: Vec<u8> = vec![
            0x67, // NAL header byte (SPS)
            0x64, // profile_idc=100
            0x00, // constraint_flags
            0x0D, // level_idc=13
            0xAD, 0xC8, 0xBF, 0xFE, 0x03, 0xC1, 0x41, 0xF9,
        ];

        let info = decode_avc_sps(&sps_nal).unwrap();
        assert_eq!(info.profile_idc, 100);
        assert_eq!(info.level_idc, 13);
        assert_eq!(info.chroma_format_idc, 1);
        assert_eq!(info.bit_depth_luma, 8);
        assert_eq!(info.bit_depth_chroma, 8);
        assert_eq!(info.width, 320);
        assert_eq!(info.height, 240);
        assert!(info.frame_mbs_only);
    }
}
