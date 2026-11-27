#![no_main]

use broadcast_bitstream::Hdr10PlusMetadata;
use broadcast_core::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;

// Fuzz the HDR10+ (SMPTE ST 2094-40) T.35 payload parser, its serializer and
// the SEI walkers that find the message in a NAL unit, an Annex B access unit
// and a length-prefixed sample. They run on untrusted broadcast input and must
// not panic; a parsed message must re-serialize to bytes that parse back.
fuzz_target!(|data: &[u8]| {
    if let Ok(meta) = Hdr10PlusMetadata::parse(data) {
        let _ = meta.num_windows();
        if let Ok(bytes) = meta.try_to_bytes() {
            let _ = Hdr10PlusMetadata::parse(&bytes);
        }
        let _ = meta.to_hevc_prefix_sei_nal();
    }
    let _ = Hdr10PlusMetadata::hevc_sei_nal_payloads(data, &mut broadcast_bitstream::Findings::new());
    let _ = Hdr10PlusMetadata::from_hevc_sei_nal(data, &mut broadcast_bitstream::Findings::new());
    let _ = Hdr10PlusMetadata::hevc_sei_nal_carries_hdr10plus(data);
    let _ = Hdr10PlusMetadata::annexb_access_unit_carries_hdr10plus(data);
    let _ = Hdr10PlusMetadata::sample_carries_hdr10plus(data);
});
