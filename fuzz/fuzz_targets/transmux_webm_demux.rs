#![no_main]

use libfuzzer_sys::fuzz_target;
use transmux::webm_demux::WebmDemux;

fuzz_target!(|data: &[u8]| {
    let _ = WebmDemux::new().demux(data);
});
