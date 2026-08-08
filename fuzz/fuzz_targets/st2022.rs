#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;
use st2022::PayloadHeader;

fuzz_target!(|data: &[u8]| {
    // ST 2022-6 HBRMT Payload Header — SMPTE ST 2022-6:2012 §6.4
    // Parse and byte-exact round-trip.
    if let Ok(header) = PayloadHeader::parse(data) {
        let reserialized = header.to_bytes();
        if let Ok(reparsed) = PayloadHeader::parse(&reserialized) {
            assert_eq!(
                reserialized, reparsed.to_bytes(),
                "PayloadHeader roundtrip mismatch"
            );
        }
    }
});
