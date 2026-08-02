#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz `media_doctor::check_hls_playlist` — the validator built on top of
// `broadcast_hls::MediaPlaylist::parse` / `MasterPlaylist::parse` — on
// arbitrary UTF-8 text. Must not panic on any input, however malformed or
// adversarial.
fuzz_target!(|data: &[u8]| {
    // Cap input size at 1 MiB to avoid unbounded allocation in the fuzzer.
    if data.len() > 1_048_576 {
        return;
    }
    if let Ok(s) = core::str::from_utf8(data) {
        let mut report = media_doctor::Report::new();
        media_doctor::check_hls_playlist(s, &mut report);
        // Consume the report to exercise the accessor (no uninitialised reads).
        let _ = report.len();
        let _ = report.findings();
    }
});
