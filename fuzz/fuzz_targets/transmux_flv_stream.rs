#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz the streaming FLV demuxer on arbitrary bytes (#738). The demuxer
// accepts untrusted remote streaming input and must not panic on any data,
// however truncated or malformed.
fuzz_target!(|data: &[u8]| {
    let mut d = transmux::StreamingFlvDemux::new();
    let _ = d.feed(data);
    while d.poll_event().is_some() {}
    d.finish();
});
