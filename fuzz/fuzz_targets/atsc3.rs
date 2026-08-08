#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // LLS binary envelope parse and round-trip, followed by SLT XML parse
    // if the data is valid UTF-8 (SLT is XML carried in the LLS payload).

    // Test 1: LLS envelope binary parse + round-trip
    if let Ok(envelope) = atsc3::LlsEnvelope::parse(data) {
        let reserialized = envelope.to_bytes();
        if let Ok(reparsed) = atsc3::LlsEnvelope::parse(&reserialized) {
            assert_eq!(
                reserialized, reparsed.to_bytes(),
                "LlsEnvelope roundtrip mismatch"
            );
        }
    }

    // Test 2: SLT XML parse (if data is valid UTF-8)
    if let Ok(xml) = core::str::from_utf8(data) {
        // Cap to 256 KiB to prevent OOM from deeply nested XML
        if xml.len() <= 256 * 1024 {
            let _ = atsc3::Slt::parse(xml);
        }
    }
});
