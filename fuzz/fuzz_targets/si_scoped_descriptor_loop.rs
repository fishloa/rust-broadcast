#![no_main]

use libfuzzer_sys::fuzz_target;

// INT / UNT / RNT scoped descriptor namespaces (issue #1285): every loop walks
// without panicking and every typed entry re-serializes without panicking.
fuzz_target!(|data: &[u8]| {
    for item in dvb_si::descriptors::parse_int_loop(data).flatten() {
        let _ = item.reserialize();
    }
    for item in dvb_si::descriptors::parse_unt_loop(data).flatten() {
        let _ = item.reserialize();
    }
    for item in dvb_si::descriptors::parse_rnt_loop(data).flatten() {
        let _ = item.reserialize();
    }
});
