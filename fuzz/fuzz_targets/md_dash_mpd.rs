#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz `media_doctor::check_dash_mpd` — the validator built on top of
// `transmux::Mpd::parse` — on arbitrary UTF-8 text. Must not panic on any
// input, however malformed or adversarial (including invalid XML, truncated
// documents, and legitimate MPDs with valid spec constructs like a first
// `<S>` without `@t`).
fuzz_target!(|data: &[u8]| {
    // Cap input size at 1 MiB to avoid unbounded allocation in the fuzzer.
    if data.len() > 1_048_576 {
        return;
    }
    if let Ok(s) = core::str::from_utf8(data) {
        let mut report = media_doctor::Report::new();
        media_doctor::check_dash_mpd(s, &mut report);
        let _ = report.len();
        let _ = report.findings();
    }
});
