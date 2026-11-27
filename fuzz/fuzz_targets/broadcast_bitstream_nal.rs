#![no_main]

use broadcast_bitstream::au::{AccessUnitSplitter, split_access_units};
use broadcast_bitstream::annexb::{
    annexb_to_length_prefixed, iter_annexb_nals, iter_length_prefixed_nals,
    iter_length_prefixed_nals_with, length_prefixed_to_annexb, normalise_nal_length_size,
};
use broadcast_bitstream::nal::{
    NalCodec, access_unit_is_idr, access_unit_is_keyframe, access_unit_is_rap,
    avc_access_unit_parameter_sets, caption_cc_data, is_idr_nal, is_keyframe_nal, nal_unit_type,
    recovery_point_sei,
};
use libfuzzer_sys::fuzz_target;

// Fuzz the NAL, Annex B and access-unit layer on arbitrary bytes.
//
// Every function here runs on untrusted broadcast input: it must return, never
// panic, and the splitter's open-NAL bound keeps a hostile stream from growing
// its buffer without limit.
fuzz_target!(|data: &[u8]| {
    let annexb_nals: Vec<&[u8]> = iter_annexb_nals(data).collect();
    let _ = annexb_to_length_prefixed(data);
    let _ = iter_length_prefixed_nals(data);
    for size in 0usize..=5 {
        let _ = iter_length_prefixed_nals_with(data, size);
        let _ = normalise_nal_length_size(data, size);
    }
    let _ = length_prefixed_to_annexb(data);

    for codec in [NalCodec::Avc, NalCodec::Hevc, NalCodec::Vvc] {
        for nal in &annexb_nals {
            let _ = nal_unit_type(codec, nal);
            let _ = is_keyframe_nal(codec, nal);
            let _ = is_idr_nal(codec, nal);
            let _ = recovery_point_sei(nal);
        }
        for length_prefixed in [false, true] {
            let _ = access_unit_is_idr(codec, data);
            let _ = access_unit_is_keyframe(codec, data, length_prefixed);
            let _ = access_unit_is_rap(codec, data, length_prefixed);
            let _ = caption_cc_data(codec, data, length_prefixed, &mut broadcast_bitstream::Findings::new());
        }

        let _ = split_access_units(codec, data, &mut broadcast_bitstream::Findings::new());
        // A small caller-supplied bound: a hostile stream that never closes a
        // NAL must be rejected, not buffered without limit.
        let mut splitter = AccessUnitSplitter::with_max_open_nal_bytes(codec, 1 << 20);
        for chunk in data.chunks(64) {
            if splitter.push(chunk).is_err() {
                break;
            }
            while splitter.pop().is_some() {}
        }
        splitter.finish();
        while splitter.pop().is_some() {}
    }

    let _ = avc_access_unit_parameter_sets(data);
});
