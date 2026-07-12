#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;
use st337::Burst;

fuzz_target!(|data: &[u8]| {
    if let Ok(burst) = Burst::parse(data) {
        let serialized = burst.to_bytes();
        if let Ok(reparsed) = Burst::parse(&serialized) {
            assert_eq!(
                serialized,
                reparsed.to_bytes(),
                "st337 burst roundtrip mismatch"
            );
        }
    }
});
