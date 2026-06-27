#![no_main]

use dvb_common::Parse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = scte35_splice::SpliceInfoSection::parse(data);
});
