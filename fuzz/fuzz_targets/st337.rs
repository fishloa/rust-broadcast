#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;
use st337::Burst;

fuzz_target!(|data: &[u8]| {
    if let Ok(burst) = Burst::parse(data) {
        let serialized = burst.try_to_bytes().unwrap();
        if let Ok(reparsed) = Burst::parse(&serialized) {
            assert_eq!(
                serialized,
                reparsed.try_to_bytes().unwrap(),
                "st337 burst roundtrip mismatch"
            );
        }
    }
});
