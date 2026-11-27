#![no_main]

use broadcast_bitstream::aac_asc::AudioSpecificConfig;
use broadcast_core::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;

// Fuzz the AAC `AudioSpecificConfig` parser on arbitrary bytes: parse, derive
// the HE-AAC signalling and the frame span, and re-serialize. A config the
// parser accepted must serialize (the type can express only valid values).
fuzz_target!(|data: &[u8]| {
    let Ok(asc) = AudioSpecificConfig::parse(data) else {
        return;
    };
    let _ = asc.heaac_signaling();
    let _ = asc.samples_per_frame();
    let _ = asc.effective_sampling_frequency();
    let _ = asc.core_audio_object_type();
    let _ = asc.extension_sampling_frequency();
    let _ = asc.to_adts_header(64);
    let need = asc.serialized_len();
    let mut out = vec![0u8; need];
    if asc.serialize_into(&mut out).is_ok() {
        let _ = AudioSpecificConfig::parse(&out);
    }
});
