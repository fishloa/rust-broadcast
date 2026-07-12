#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;
use st12_1::LtcFrame;

fuzz_target!(|data: &[u8]| {
    // SMPTE ST 12-1:2014 §9.2 80-bit LTC codeword: parse + byte-identical
    // round trip.
    if let Ok(frame) = LtcFrame::parse(data) {
        let serialized = frame.to_bytes();
        if let Ok(reparsed) = LtcFrame::parse(&serialized) {
            assert_eq!(
                serialized,
                reparsed.to_bytes(),
                "LTC frame roundtrip mismatch"
            );
        }
    }
});
