#![no_main]

use broadcast_bitstream::rfc6381::{rfc6381_avc1, rfc6381_mp4a, rfc6381_vvc1};
use broadcast_bitstream::sps::{
    decode_avc_display_facts, decode_avc_reorder_depth, decode_avc_slice_sps, decode_avc_sps,
    decode_avc_vui_timing, decode_hevc_display_facts, decode_hevc_interlace_sps, decode_hevc_sps,
    decode_hevc_vps_timing, decode_hevc_vui_timing, decode_vvc_sps,
};
use libfuzzer_sys::fuzz_target;

// Fuzz every sequence-parameter-set decoder on arbitrary bytes.
//
// An SPS arrives straight off the wire, so each walk (VUI timing, HRD, inter
// reference-picture-set prediction, scaling lists, the H.265 profile_tier_level
// and the H.266 general_constraints_info) must return a typed result for any
// input: never a panic, never an unbounded loop.
fuzz_target!(|data: &[u8]| {
    let avc = decode_avc_sps(data);
    let _ = decode_avc_vui_timing(data);
    let _ = decode_avc_reorder_depth(data);
    let _ = decode_avc_slice_sps(data);
    let _ = decode_avc_display_facts(data);
    let _ = decode_hevc_sps(data);
    let _ = decode_hevc_vui_timing(data);
    let _ = decode_hevc_vps_timing(data);
    let _ = decode_hevc_interlace_sps(data);
    let _ = decode_hevc_display_facts(data);
    let _ = decode_vvc_sps(data);

    if let Ok(info) = avc {
        let _ = rfc6381_avc1(info.profile_idc(), info.constraint_flags(), info.level_idc());
    }
    let _ = rfc6381_mp4a(data.first().copied().unwrap_or_default());
    let _ = rfc6381_vvc1(
        data.first().copied().unwrap_or_default(),
        data.len() % 2 == 0,
        data.get(1).copied().unwrap_or_default(),
        data,
        data.get(2).copied().unwrap_or_default(),
    );
});
