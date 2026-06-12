#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for item in dvb_si::descriptors::parse_loop(data) {
        let _ = item;
    }
});
