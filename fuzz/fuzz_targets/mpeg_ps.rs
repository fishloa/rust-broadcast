#![no_main]

use dvb_common::Parse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = mpeg_ps::PackHeader::parse(data);
    let _ = mpeg_ps::program_stream::parse_pack(data);
});
