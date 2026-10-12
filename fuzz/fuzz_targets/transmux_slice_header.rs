#![no_main]

use libfuzzer_sys::fuzz_target;
use transmux::slice_header::AvcParamSets;

// Fuzz the H.264 slice-header / PPS / SPS prefix parsers and the PAFF field
// classifier (#1161) on arbitrary Annex B bytes. They run on every video access
// unit of untrusted TS input and must not panic.
fuzz_target!(|data: &[u8]| {
    let mut ps = AvcParamSets::new();
    // Twice: the second pass sees whatever parameter sets the first one cached.
    let _ = ps.classify_au(data);
    let _ = ps.classify_au(data);
});
