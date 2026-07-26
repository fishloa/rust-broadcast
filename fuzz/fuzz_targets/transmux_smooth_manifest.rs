#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz the Smooth Streaming manifest parser on arbitrary UTF-8 text (#738).
// The parser handles untrusted remote XML and must not panic on any input,
// however malformed.
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = transmux::SmoothManifest::parse(s);
    }
});
