#![no_main]

use libfuzzer_sys::fuzz_target;
use transmux::sps::{decode_avc_slice_sps, decode_hevc_interlace_sps};
use transmux::{LiveJoin, NalCodec, Sample};

// Fuzz the interlace-facts SPS walkers (H.264 VUI/HRD, H.265 inter-RPS + VUI,
// #1162 R3) and the live-join state machine (R4, SEI/NAL scanning) on
// arbitrary bytes. All run on untrusted TS input and must not panic or hang.
fuzz_target!(|data: &[u8]| {
    let _ = decode_avc_slice_sps(data);
    let _ = decode_hevc_interlace_sps(data);
    for (codec, length_size) in [
        (NalCodec::Avc, 4),
        (NalCodec::Avc, 2),
        (NalCodec::Hevc, 4),
        (NalCodec::Vvc, 1),
    ] {
        let mut j = LiveJoin::new(codec, length_size);
        let s = Sample::new(data.to_vec(), Some(1), Some(1), None, false);
        let _ = j.push(&s);
        let _ = j.push(&s);
        j.rejoin();
    }
});
