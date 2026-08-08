#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // RIST Simple Profile (VSF TR-06-1:2020) RTCP message types:
    // GenericNack, RangeNack, and RttEcho all implement Parse/Serialize.
    // Fuzz each with byte-exact round-trip validation.

    // Test 1: GenericNack (RFC 4585 §6.2.1)
    if let Ok(nack) = rist_runtime::GenericNack::parse(data) {
        let reserialized = nack.to_bytes();
        if let Ok(reparsed) = rist_runtime::GenericNack::parse(&reserialized) {
            assert_eq!(
                reserialized, reparsed.to_bytes(),
                "GenericNack roundtrip mismatch"
            );
        }
    }

    // Test 2: RangeNack (RIST-specific RTCP APP)
    if let Ok(nack) = rist_runtime::RangeNack::parse(data) {
        let reserialized = nack.to_bytes();
        if let Ok(reparsed) = rist_runtime::RangeNack::parse(&reserialized) {
            assert_eq!(
                reserialized, reparsed.to_bytes(),
                "RangeNack roundtrip mismatch"
            );
        }
    }

    // Test 3: RttEcho (RTCP APP for round-trip time measurement)
    if let Ok(echo) = rist_runtime::RttEcho::parse(data) {
        let reserialized = echo.to_bytes();
        if let Ok(reparsed) = rist_runtime::RttEcho::parse(&reserialized) {
            assert_eq!(reserialized, reparsed.to_bytes(), "RttEcho roundtrip mismatch");
        }
    }
});
