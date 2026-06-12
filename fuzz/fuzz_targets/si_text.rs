#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = dvb_si::text::decode(data);
    let _ = dvb_si::text::DvbText::new(data).decode();
});
