#![no_main]

use broadcast_common::Unpackage;
use libfuzzer_sys::fuzz_target;
use transmux::ProgressiveDemux;

const MAX_BYTES: usize = 1 << 24;

fuzz_target!(|data: &[u8]| {
    if let Ok(mut demux) = ProgressiveDemux::new(MAX_BYTES) {
        let _ = demux.unpackage(data);
    }
});
