#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;
use rdd29::AtmosFrame;

fuzz_target!(|data: &[u8]| {
    // SMPTE RDD 29:2019 ATMOSFrame element: parse + byte-identical round trip.
    if let Ok(frame) = AtmosFrame::parse(data) {
        let serialized = frame.to_bytes();
        if let Ok(reparsed) = AtmosFrame::parse(&serialized) {
            assert_eq!(
                serialized,
                reparsed.to_bytes(),
                "ATMOSFrame roundtrip mismatch"
            );
        }
    }
});
