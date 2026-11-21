//! Synthetic H.264/H.265 SPS bit-stream builders for the audit-23 bounds tests.
//!
//! The builders emit exactly the syntax the decoders walk (ITU-T H.264
//! §7.3.2.1.1 / §E.1.1, ITU-T H.265 §7.3.2.2.1 / §E.2.1 and the
//! `st_ref_pic_set()` / `scaling_list_data()` / `hrd_parameters()` sub-walks),
//! so a test can place a specific field value and observe how far the decoder
//! got by reading a fact encoded after it.

use crate::bit_writer::BitWriter;

/// `hrd_parameters()` fields for an AVC VUI (nal HRD only).
pub(crate) struct AvcHrd {
    /// `cpb_cnt_minus1`; the builder emits `cpb_cnt_minus1 + 1` entries.
    pub cpb_cnt_minus1: u64,
}

/// The AVC VUI fields the tests drive; everything else is written as absent.
pub(crate) struct AvcVui {
    /// `(num_units_in_tick, time_scale)`; `None` writes the flag 0.
    pub timing: Option<(u32, u32)>,
    /// `nal_hrd_parameters()`; `None` writes the flag 0.
    pub hrd: Option<AvcHrd>,
    /// `(max_num_reorder_frames, max_dec_frame_buffering)`.
    pub restriction: Option<(u32, u32)>,
}

/// H.264 SPS layout knobs (defaults are 720x576 4:2:0 progressive High).
pub(crate) struct AvcSps {
    pub pic_order_cnt_type: u64,
    /// `num_ref_frames_in_pic_order_cnt_cycle` (POC type 1 only).
    pub num_ref_cycle: u64,
    pub pic_width_in_mbs_minus1: u64,
    pub pic_height_in_map_units_minus1: u64,
    pub frame_mbs_only: bool,
    /// `(left, right, top, bottom)` in crop units; all zero omits the window.
    pub crop: (u64, u64, u64, u64),
    pub vui: Option<AvcVui>,
}

impl Default for AvcSps {
    fn default() -> Self {
        Self {
            pic_order_cnt_type: 0,
            num_ref_cycle: 0,
            pic_width_in_mbs_minus1: 44,
            pic_height_in_map_units_minus1: 35,
            frame_mbs_only: true,
            crop: (0, 0, 0, 0),
            vui: None,
        }
    }
}

/// Build an H.264 SPS NAL unit (1-byte NAL header included), High profile.
pub(crate) fn avc_sps(o: &AvcSps) -> Vec<u8> {
    let mut nal = vec![0x67u8, 100, 0x00, 30];
    let mut s = BitWriter::new();
    s.ue(0); // seq_parameter_set_id
    s.ue(1); // chroma_format_idc 4:2:0
    s.ue(0); // bit_depth_luma_minus8
    s.ue(0); // bit_depth_chroma_minus8
    s.bit(false); // qpprime_y_zero_transform_bypass_flag
    s.bit(false); // seq_scaling_matrix_present_flag
    s.ue(0); // log2_max_frame_num_minus4
    s.ue(o.pic_order_cnt_type);
    match o.pic_order_cnt_type {
        0 => {
            s.ue(0); // log2_max_pic_order_cnt_lsb_minus4
        }
        1 => {
            s.bit(false); // delta_pic_order_always_zero_flag
            s.se(0); // offset_for_non_ref_pic
            s.se(0); // offset_for_top_to_bottom_field
            s.ue(o.num_ref_cycle);
            for _ in 0..o.num_ref_cycle {
                s.se(1); // offset_for_ref_frame[i]
            }
        }
        _ => {}
    }
    s.ue(1); // max_num_ref_frames
    s.bit(false); // gaps_in_frame_num_value_allowed_flag
    s.ue(o.pic_width_in_mbs_minus1);
    s.ue(o.pic_height_in_map_units_minus1);
    s.bit(o.frame_mbs_only);
    if !o.frame_mbs_only {
        s.bit(false); // mb_adaptive_frame_field_flag
    }
    s.bit(true); // direct_8x8_inference_flag
    let crop_present = o.crop != (0, 0, 0, 0);
    s.bit(crop_present);
    if crop_present {
        s.ue(o.crop.0);
        s.ue(o.crop.1);
        s.ue(o.crop.2);
        s.ue(o.crop.3);
    }
    match &o.vui {
        None => {
            s.bit(false); // vui_parameters_present_flag
        }
        Some(v) => {
            s.bit(true);
            write_avc_vui(&mut s, v);
        }
    }
    s.bit(true); // rbsp_stop_one_bit
    nal.extend(escape(&s.finish()));
    nal
}

fn write_avc_vui(s: &mut BitWriter, v: &AvcVui) {
    s.bit(false); // aspect_ratio_info_present_flag
    s.bit(false); // overscan_info_present_flag
    s.bit(false); // video_signal_type_present_flag
    s.bit(false); // chroma_loc_info_present_flag
    match v.timing {
        None => {
            s.bit(false);
        }
        Some((n, ts)) => {
            s.bit(true);
            s.u(32, u64::from(n));
            s.u(32, u64::from(ts));
            s.bit(false); // fixed_frame_rate_flag
        }
    }
    match &v.hrd {
        None => {
            s.bit(false); // nal_hrd_parameters_present_flag
        }
        Some(h) => {
            s.bit(true);
            write_avc_hrd(s, h.cpb_cnt_minus1);
        }
    }
    s.bit(false); // vcl_hrd_parameters_present_flag
    if v.hrd.is_some() {
        s.bit(false); // low_delay_hrd_flag
    }
    s.bit(false); // pic_struct_present_flag
    match v.restriction {
        None => {
            s.bit(false);
        }
        Some((reorder, buffering)) => {
            s.bit(true);
            s.bit(true); // motion_vectors_over_pic_boundaries_flag
            s.ue(0); // max_bytes_per_pic_denom
            s.ue(0); // max_bits_per_mb_denom
            s.ue(0); // log2_max_mv_length_horizontal
            s.ue(0); // log2_max_mv_length_vertical
            s.ue(u64::from(reorder));
            s.ue(u64::from(buffering));
        }
    }
}

fn write_avc_hrd(s: &mut BitWriter, cpb_cnt_minus1: u64) {
    s.ue(cpb_cnt_minus1);
    s.u(4, 0); // bit_rate_scale
    s.u(4, 0); // cpb_size_scale
    for _ in 0..=cpb_cnt_minus1 {
        s.ue(0); // bit_rate_value_minus1
        s.ue(0); // cpb_size_value_minus1
        s.bit(false); // cbr_flag
    }
    s.u(5, 0); // initial_cpb_removal_delay_length_minus1
    s.u(5, 0); // cpb_removal_delay_length_minus1
    s.u(5, 0); // dpb_output_delay_length_minus1
    s.u(5, 0); // time_offset_length
}

/// One sub-layer of an HEVC `hrd_parameters()`.
pub(crate) struct HevcHrdLayer {
    pub general: bool,
    pub within: bool,
    pub low_delay: bool,
    pub cpb_minus1: u64,
    pub elemental: u64,
}

/// HEVC `hrd_parameters()` knobs.
pub(crate) struct HevcHrd {
    pub nal: bool,
    pub vcl: bool,
    pub sub_pic: bool,
    pub layers: Vec<HevcHrdLayer>,
}

/// One `st_ref_pic_set()`: explicit when `inter` is false, else predicted from
/// the preceding set.
pub(crate) struct HevcRps {
    pub inter: bool,
    pub deltas_neg: Vec<u64>,
    pub deltas_pos: Vec<u64>,
    pub delta_rps_sign: bool,
    pub abs_delta_rps_minus1: u64,
    /// `used_by_curr_pic_flag[j]` for `j in 0..=NumDeltaPocs`.
    pub used: Vec<bool>,
    /// `use_delta_flag[j]`, read only where `used[j]` is false.
    pub use_delta: Vec<bool>,
}

impl HevcRps {
    pub(crate) fn explicit(deltas_neg: Vec<u64>, deltas_pos: Vec<u64>) -> Self {
        Self {
            inter: false,
            deltas_neg,
            deltas_pos,
            delta_rps_sign: false,
            abs_delta_rps_minus1: 0,
            used: Vec::new(),
            use_delta: Vec::new(),
        }
    }
}

/// The HEVC VUI fields the tests drive; the rest are written as absent.
pub(crate) struct HevcVui {
    pub field_seq: bool,
    pub frame_field_info_present: bool,
    pub default_display: Option<(u64, u64, u64, u64)>,
    pub timing: Option<(u32, u32)>,
    pub hrd: Option<HevcHrd>,
}

/// H.265 SPS layout knobs (defaults are a 320x240 Main-profile SPS, no VUI).
pub(crate) struct HevcSps {
    pub max_sub_layers_minus1: u8,
    pub sub_layer_profile_present: Vec<bool>,
    pub sub_layer_level_present: Vec<bool>,
    pub profile_idc: u8,
    pub compatibility_flags: u32,
    pub constraint_flags: u64,
    pub level_idc: u8,
    pub chroma_format_idc: u64,
    pub width: u64,
    pub height: u64,
    pub conformance: Option<(u64, u64, u64, u64)>,
    pub log2_max_poc_lsb_minus4: u64,
    /// One `scaling_list_pred_mode_flag` per matrix, size_id 0..3 in order.
    pub scaling_list: Option<Vec<bool>>,
    pub rps: Vec<HevcRps>,
    /// `(num_long_term_ref_pics_sps)`: entries are 4-bit + flag wide when the
    /// main `log2_max_poc_lsb_minus4` is 0.
    pub long_term_ref_pics: Option<u64>,
    pub vui: Option<HevcVui>,
}

impl Default for HevcSps {
    fn default() -> Self {
        Self {
            max_sub_layers_minus1: 0,
            sub_layer_profile_present: Vec::new(),
            sub_layer_level_present: Vec::new(),
            profile_idc: 1,
            compatibility_flags: 0,
            constraint_flags: 0,
            level_idc: 93,
            chroma_format_idc: 1,
            width: 320,
            height: 240,
            conformance: None,
            log2_max_poc_lsb_minus4: 0,
            scaling_list: None,
            rps: Vec::new(),
            long_term_ref_pics: None,
            vui: None,
        }
    }
}

/// Build an H.265 SPS NAL unit (2-byte NAL header included).
pub(crate) fn hevc_sps(o: &HevcSps) -> Vec<u8> {
    let mut nal = vec![0x42u8, 0x01];
    let mut s = BitWriter::new();
    let max = o.max_sub_layers_minus1;
    s.u(4, 0); // sps_video_parameter_set_id
    s.u(3, u64::from(max));
    s.bit(true); // sps_temporal_id_nesting_flag
    s.u(2, 0); // general_profile_space
    s.bit(false); // general_tier_flag
    s.u(5, u64::from(o.profile_idc));
    s.u(32, u64::from(o.compatibility_flags));
    s.u(48, o.constraint_flags);
    s.u(8, u64::from(o.level_idc));
    for i in 0..usize::from(max) {
        s.bit(o.sub_layer_profile_present[i]);
        s.bit(o.sub_layer_level_present[i]);
    }
    if max > 0 {
        for _ in max..8 {
            s.u(2, 0); // reserved_zero_2bits
        }
    }
    for i in 0..usize::from(max) {
        if o.sub_layer_profile_present[i] {
            s.u(2, 0); // sub_layer_profile_space
            s.bit(false); // sub_layer_tier_flag
            s.u(5, 1); // sub_layer_profile_idc
            s.u(32, 0); // sub_layer_profile_compatibility_flags
            s.u(48, 0); // sub_layer_constraint_indicator_flags
        }
        if o.sub_layer_level_present[i] {
            s.u(8, u64::from(o.level_idc));
        }
    }
    s.ue(0); // sps_seq_parameter_set_id
    s.ue(o.chroma_format_idc);
    if o.chroma_format_idc == 3 {
        s.bit(false); // separate_colour_plane_flag
    }
    s.ue(o.width);
    s.ue(o.height);
    match o.conformance {
        None => {
            s.bit(false);
        }
        Some(c) => {
            s.bit(true);
            s.ue(c.0);
            s.ue(c.1);
            s.ue(c.2);
            s.ue(c.3);
        }
    }
    s.ue(0); // bit_depth_luma_minus8
    s.ue(0); // bit_depth_chroma_minus8
    s.ue(o.log2_max_poc_lsb_minus4);
    s.bit(false); // sps_sub_layer_ordering_info_present_flag
    s.ue(0); // sps_max_dec_pic_buffering_minus1
    s.ue(0); // sps_max_num_reorder_pics
    s.ue(0); // sps_max_latency_increase_plus1
    s.ue(0); // log2_min_luma_coding_block_size_minus3
    s.ue(0); // log2_diff_max_min_luma_coding_block_size
    s.ue(0); // log2_min_luma_transform_block_size_minus2
    s.ue(0); // log2_diff_max_min_luma_transform_block_size
    s.ue(0); // max_transform_hierarchy_depth_inter
    s.ue(0); // max_transform_hierarchy_depth_intra
    match &o.scaling_list {
        None => {
            s.bit(false); // scaling_list_enabled_flag
        }
        Some(modes) => {
            s.bit(true);
            s.bit(true); // sps_scaling_list_data_present_flag
            write_hevc_scaling_list(&mut s, modes);
        }
    }
    s.bit(false); // amp_enabled_flag
    s.bit(false); // sample_adaptive_offset_enabled_flag
    s.bit(false); // pcm_enabled_flag
    s.ue(o.rps.len() as u64);
    for (i, rps) in o.rps.iter().enumerate() {
        write_hevc_rps(&mut s, i, rps);
    }
    match o.long_term_ref_pics {
        None => {
            s.bit(false);
        }
        Some(n) => {
            s.bit(true);
            s.ue(n);
            let poc_bits = (o.log2_max_poc_lsb_minus4 + 4) as u32;
            for _ in 0..n {
                s.u(poc_bits, 1); // lt_ref_pic_poc_lsb_sps
                s.bit(true); // used_by_curr_pic_lt_sps_flag
            }
        }
    }
    s.bit(false); // sps_temporal_mvp_enabled_flag
    s.bit(false); // strong_intra_smoothing_enabled_flag
    match &o.vui {
        None => {
            s.bit(false); // vui_parameters_present_flag
        }
        Some(v) => {
            s.bit(true);
            write_hevc_vui(&mut s, v);
        }
    }
    s.bit(true); // rbsp_stop_one_bit
    nal.extend(escape(&s.finish()));
    nal
}

fn write_hevc_scaling_list(s: &mut BitWriter, modes: &[bool]) {
    let mut idx = 0usize;
    for size_id in 0..4u32 {
        let matrix_count = if size_id == 3 { 2 } else { 6 };
        for _ in 0..matrix_count {
            let pred = modes[idx];
            idx += 1;
            s.bit(pred);
            if !pred {
                s.ue(0); // scaling_list_pred_matrix_id_delta
            } else {
                let coef_num = core::cmp::min(64u32, 1 << (4 + (size_id << 1)));
                if size_id > 1 {
                    s.ue(0); // scaling_list_dc_coef_minus8
                }
                for _ in 0..coef_num {
                    s.ue(1); // scaling_list_delta_coef
                }
            }
        }
    }
}

fn write_hevc_rps(s: &mut BitWriter, index: usize, rps: &HevcRps) {
    if index != 0 {
        s.bit(rps.inter); // inter_ref_pic_set_prediction_flag
    }
    if rps.inter {
        s.bit(rps.delta_rps_sign);
        s.ue(rps.abs_delta_rps_minus1);
        for j in 0..rps.used.len() {
            s.bit(rps.used[j]);
            if !rps.used[j] {
                s.bit(rps.use_delta[j]);
            }
        }
    } else {
        s.ue(rps.deltas_neg.len() as u64);
        s.ue(rps.deltas_pos.len() as u64);
        for &d in &rps.deltas_neg {
            s.ue(d); // delta_poc_s0_minus1
            s.bit(true); // used_by_curr_pic_s0_flag
        }
        for &d in &rps.deltas_pos {
            s.ue(d); // delta_poc_s1_minus1
            s.bit(true); // used_by_curr_pic_s1_flag
        }
    }
}

fn write_hevc_vui(s: &mut BitWriter, v: &HevcVui) {
    s.bit(false); // aspect_ratio_info_present_flag
    s.bit(false); // overscan_info_present_flag
    s.bit(false); // video_signal_type_present_flag
    s.bit(false); // chroma_loc_info_present_flag
    s.bit(false); // neutral_chroma_indication_flag
    s.bit(v.field_seq);
    s.bit(v.frame_field_info_present);
    match v.default_display {
        None => {
            s.bit(false);
        }
        Some(d) => {
            s.bit(true);
            s.ue(d.0);
            s.ue(d.1);
            s.ue(d.2);
            s.ue(d.3);
        }
    }
    match v.timing {
        None => {
            s.bit(false);
        }
        Some((n, ts)) => {
            s.bit(true);
            s.u(32, u64::from(n));
            s.u(32, u64::from(ts));
            s.bit(false); // vui_poc_proportional_to_timing_flag
            match &v.hrd {
                None => {
                    s.bit(false);
                }
                Some(h) => {
                    s.bit(true);
                    write_hevc_hrd(s, h);
                }
            }
        }
    }
}

fn write_hevc_hrd(s: &mut BitWriter, h: &HevcHrd) {
    s.bit(h.nal);
    s.bit(h.vcl);
    if h.nal || h.vcl {
        s.bit(h.sub_pic);
        if h.sub_pic {
            s.u(8, 0); // tick_divisor_minus2
            s.u(5, 0); // du_cpb_removal_delay_increment_length_minus1
            s.bit(false); // sub_pic_cpb_params_in_pic_timing_sei_flag
            s.u(5, 0); // dpb_output_delay_du_length_minus1
        }
        s.u(4, 0); // bit_rate_scale
        s.u(4, 0); // cpb_size_scale
        if h.sub_pic {
            s.u(4, 0); // cpb_size_du_scale
        }
        s.u(5, 0); // initial_cpb_removal_delay_length_minus1
        s.u(5, 0); // au_cpb_removal_delay_length_minus1
        s.u(5, 0); // dpb_output_delay_length_minus1
    }
    for layer in &h.layers {
        s.bit(layer.general);
        let within = layer.general || layer.within;
        if !layer.general {
            s.bit(layer.within);
        }
        let low_delay = if within { false } else { layer.low_delay };
        if !within {
            s.bit(layer.low_delay);
        }
        if within {
            s.ue(layer.elemental);
        }
        let cpb = if low_delay {
            1
        } else {
            s.ue(layer.cpb_minus1);
            layer.cpb_minus1 + 1
        };
        if h.nal {
            write_hevc_sub_layer_hrd(s, cpb, h.sub_pic);
        }
        if h.vcl {
            write_hevc_sub_layer_hrd(s, cpb, h.sub_pic);
        }
    }
}

fn write_hevc_sub_layer_hrd(s: &mut BitWriter, cpb_cnt: u64, sub_pic: bool) {
    for _ in 0..cpb_cnt {
        s.ue(0); // bit_rate_value_minus1
        s.ue(0); // cpb_size_value_minus1
        if sub_pic {
            s.ue(0); // cpb_size_du_value_minus1
            s.ue(0); // bit_rate_du_value_minus1
        }
        s.bit(false); // cbr_flag
    }
}

/// Insert `emulation_prevention_three_byte` before any byte `<= 0x03` that
/// follows a `00 00` pair (ITU-T H.264 §7.4.1 / H.265 §7.4.2), so the
/// decoder's `00 00 03` removal restores exactly the written RBSP.
fn escape(rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rbsp.len());
    let mut zeros = 0usize;
    for &b in rbsp {
        if zeros >= 2 && b <= 0x03 {
            out.push(0x03);
            zeros = 0;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    if zeros >= 2 {
        out.push(0x03);
    }
    out
}
