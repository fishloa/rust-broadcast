#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../support/doctor_text.rs"]
mod doctor_text;

// Fuzz `media_doctor::check_dash_mpd` — the validator built on top of
// `dash_runtime::mpd::Mpd::parse` — on arbitrary UTF-8 text. Must not panic on any
// input, however malformed or adversarial (including invalid XML, truncated
// documents, and legitimate MPDs with valid spec constructs like a first
// `<S>` without `@t`).
fuzz_target!(|data: &[u8]| doctor_text::check_text(data, media_doctor::check_dash_mpd));
