#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = st291::AncDataPacket::parse(data);
    let _ = st291::AncDataDescriptor::parse(data);
});
