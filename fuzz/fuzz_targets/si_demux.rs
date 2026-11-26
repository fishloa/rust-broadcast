#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut demux = dvb_si::demux::SiDemux::new(dvb_si::demux::SiDemuxConfig::default());
    for chunk in data.chunks(188) {
        for _event in demux.feed(chunk) {}
    }
});
