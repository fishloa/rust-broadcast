#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = dvb_si::tables::AnyTableSection::parse(data);
});
